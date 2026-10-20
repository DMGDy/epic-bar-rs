use std::{
    collections::BTreeMap,
    env,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
};

use serde::Deserialize;

pub const WORKSPACE_COUNT: usize = 9;

/* A bar-relevant Hyprland event read from socket2.
 * socket2 lines look like `EVENT>>data`; only events that require the bar
 * to refresh are represented here, everything else is ignored while reading.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HyprEvent {
    Workspace,
    ActiveWindow,
    WindowTitle,
    OpenWindow,
    CloseWindow,
    MoveWindow,
}

impl HyprEvent {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "workspace" | "workspacev2" | "focusedmon" | "focusedmonv2" => Some(Self::Workspace),
            "activewindow" | "activewindowv2" => Some(Self::ActiveWindow),
            "windowtitle" | "windowtitlev2" => Some(Self::WindowTitle),
            "openwindow" => Some(Self::OpenWindow),
            "closewindow" => Some(Self::CloseWindow),
            "movewindow" => Some(Self::MoveWindow),
            _ => None,
        }
    }
}

/* for this program:
 *  tag: the index of the workspace
 *  workspace: the actual thing containing the windows
 */

/* Window: metadata assoicated to an open window on hyprland
 *
 * name: the desktop name of the program
 * info: the secondary title or "information" of the window
 * pid: pid associated with the window open
 * tag: the workspace index th window exists on
 * order: 0 meaning active, the order of when it was used
 */
pub struct Window {
    pub name: String,
    pub info: String,
    pub address: String,
    pub class: String,
    pub tag: usize,
    pub order: usize,
}

// sorted by order (revent activity)
pub type AllWindows = Vec<Window>;

/* windows: vector of windows in order of activity
 * order: the order of this workspace in activity
 */
pub struct Workspace {
    pub windows: Vec<Window>,
    pub tag: usize,
    pub order: usize,
    pub active: bool,
}

pub type Workspaces = BTreeMap<usize, Workspace>;

fn get_hyprland_sock(num: Option<&str>) -> io::Result<UnixStream> {
    let runtime = env::var("XDG_RUNTIME_DIR").map_err(|error| {
        io::Error::new(io::ErrorKind::NotFound, format!("XDG_RUNTIME_DIR: {error}"))
    })?;
    let instance = env::var("HYPRLAND_INSTANCE_SIGNATURE").map_err(|error| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("HYPRLAND_INSTANCE_SIGNATURE: {error}"),
        )
    })?;
    UnixStream::connect(format!(
        "{runtime}/hypr/{instance}/.socket{}.sock",
        num.unwrap_or_default()
    ))
}

fn request(command: &str) -> Result<String, String> {
    let mut socket = get_hyprland_sock(None).map_err(|error| error.to_string())?;
    socket
        .write_all(command.as_bytes())
        .map_err(|error| error.to_string())?;
    let mut reply = String::new();
    socket
        .read_to_string(&mut reply)
        .map_err(|error| error.to_string())?;
    Ok(reply)
}

/* Minimal typed views over Hyprland's JSON IPC replies (the `j/` command
 * variants, e.g. `j/clients`). Only the fields the bar uses are declared;
 * serde ignores the rest. This replaces the whitespace tokenizer, which
 * could be corrupted by titles that happened to contain field-name markers.
 */
#[derive(Deserialize)]
struct JsonClient {
    address: String,
    title: String,
    class: String,
    #[serde(rename = "initialTitle")]
    initial_title: String,
    #[serde(rename = "focusHistoryID")]
    focus_history_id: i64,
    workspace: JsonWorkspaceRef,
}

#[derive(Deserialize)]
struct JsonWorkspaceRef {
    id: i64,
}

#[derive(Deserialize)]
struct JsonActiveWorkspace {
    id: i64,
}

fn get_windows() -> Result<Vec<Window>, String> {
    let reply = request("j/clients")?;
    parse_windows(&reply).map_err(|error| format!("invalid Hyprland clients: {error}"))
}

fn parse_windows(reply: &str) -> Result<Vec<Window>, serde_json::Error> {
    let clients: Vec<JsonClient> = serde_json::from_str(reply)?;
    let mut all_windows: Vec<Window> = Vec::new();

    for client in clients {
        // the bar models workspaces as positive usize tags; skip special
        // (negative id) workspaces the old usize parser could not represent
        let tag = match usize::try_from(client.workspace.id) {
            Ok(tag) => tag,
            Err(_) => continue,
        };

        // info = current title, keeping the pdf-basename special case
        let info = match client.title {
            title if title.contains(".pdf") => {
                title.split("/").last().unwrap_or(&title).to_string()
            }
            title => title,
        };

        // name = initial title, keeping the same app-name clean-ups
        let name = match client.initial_title {
            title if title.contains("Chromium") => "Chromium".to_string(),
            title if title.contains("OBS") => "OBS Studio".to_string(),
            title if title.contains(".pdf") => {
                title.split("/").last().unwrap_or(&title).to_string()
            }
            title if title.contains("WhatsApp") => "WhatsApp".to_string(),
            title => title,
        };

        // JSON addresses carry a 0x prefix; strip it so switch_window
        // (which re-adds 0x) keeps working unchanged
        let address = client
            .address
            .strip_prefix("0x")
            .unwrap_or(&client.address)
            .to_string();

        all_windows.push(Window {
            name,
            info,
            address,
            class: client.class,
            tag,
            // Hyprland uses -1 for clients that have not been focused yet.
            order: usize::try_from(client.focus_history_id).unwrap_or(usize::MAX),
        });
    }

    all_windows.sort_by(|win1, win2| win1.order.cmp(&win2.order));
    Ok(all_windows)
}

fn assign_tags_to_win(all_wins: AllWindows) -> Workspaces {
    let mut workspaces: Workspaces = BTreeMap::new();

    let mut order: usize = 0;
    for window in all_wins {
        workspaces
            .entry(window.tag)
            .or_insert((|| {
                let w = Workspace {
                    windows: Vec::new(),
                    active: window.order == 0,
                    tag: window.tag,
                    order,
                };
                order += 1;
                w
            })())
            .windows
            .push(window);
    }
    workspaces
}

/* Open a connection to Hyprland's socket2 event stream.
 * The returned reader is looped over until the socket closes; the caller
 * reconnects so events are never missed for the lifetime of the bar.
 */
pub fn event_socket() -> io::Result<BufReader<UnixStream>> {
    get_hyprland_sock(Some("2")).map(BufReader::new)
}

/* Block until the next bar-relevant event arrives on socket2.
 * Returns None if the socket closes (EOF) or errors, ending the read loop.
 * The line buffer is cleared each read so it can't accumulate.
 */
pub fn next_event(reader: &mut BufReader<UnixStream>) -> Option<HyprEvent> {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return None,
            Ok(_) => {
                let name = line.split(">>").next().unwrap_or("");
                if let Some(event) = HyprEvent::parse(name) {
                    return Some(event);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                eprintln!("epic-bar-rs: Hyprland event stream failed: {error}");
                return None;
            }
        }
    }
}

pub fn switch_window(adr: &String) {
    if let Err(error) = request(&format!(
        "dispatch hl.dsp.focus({{ window = \"address:0x{adr}\" }})"
    )) {
        eprintln!("epic-bar-rs: cannot focus window {adr}: {error}");
    }
}

pub fn switch_workspace(tag: usize) {
    if let Err(error) = request(&format!("dispatch hl.dsp.focus({{ workspace = {tag} }})")) {
        eprintln!("epic-bar-rs: cannot focus workspace {tag}: {error}");
    }
}

fn check_empty_active_workspace(workspaces: &mut Workspaces) -> Result<(), String> {
    let reply = request("j/activeworkspace")?;
    let active: JsonActiveWorkspace = serde_json::from_str(&reply)
        .map_err(|error| format!("invalid Hyprland active workspace: {error}"))?;

    let tag = match usize::try_from(active.id) {
        Ok(tag) => tag,
        Err(_) => return Ok(()),
    };

    set_active_workspace(workspaces, tag);
    Ok(())
}

fn set_active_workspace(workspaces: &mut Workspaces, tag: usize) {
    workspaces.entry(tag).or_insert_with(|| Workspace {
        windows: Vec::new(),
        active: true,
        tag,
        order: 0,
    });

    for workspace in workspaces.values_mut() {
        workspace.active = workspace.tag == tag;
        if workspace.tag != tag {
            workspace.order += 1;
        } else {
            workspace.order = 0;
        }
    }
}

pub fn get_workspaces() -> Result<Workspaces, String> {
    let all_windows = get_windows()?;
    let mut workspaces = assign_tags_to_win(all_windows);
    check_empty_active_workspace(&mut workspaces)?;

    Ok(workspaces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Shutdown;

    #[test]
    fn json_clients_preserve_titles_and_handle_unfocused_or_special_windows() {
        let clients = parse_windows(
            r#"[
            {"address":"0xabc","title":"title with workspace: ID and\nnewlines","class":"kitty",
             "initialTitle":"Terminal workspace: 5","focusHistoryID":0,"workspace":{"id":1}},
            {"address":"0xdef","title":"Unfocused","class":"kitty",
             "initialTitle":"Other","focusHistoryID":-1,"workspace":{"id":2}},
            {"address":"0xfff","title":"Special","class":"kitty",
             "initialTitle":"Scratch","focusHistoryID":1,"workspace":{"id":-99}}
        ]"#,
        )
        .unwrap();
        assert_eq!(clients.len(), 2);
        assert_eq!(clients[0].address, "abc");
        assert_eq!(clients[0].tag, 1);
        assert_eq!(clients[0].name, "Terminal workspace: 5");
        assert_eq!(clients[0].info, "title with workspace: ID and\nnewlines");
        assert_eq!(clients[1].order, usize::MAX);
        assert!(parse_windows("not JSON").is_err());
    }

    #[test]
    fn persistent_event_reader_skips_unrelated_messages_and_stops_at_eof() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        writer.write_all(b"ignored>>payload\nworkspace>>2\nactivewindow>>kitty,title\nopenwindow>>abc\nclosewindow>>abc\nmovewindow>>abc,3\n").unwrap();
        writer.shutdown(Shutdown::Write).unwrap();
        let mut reader = BufReader::new(reader);
        for event in [
            HyprEvent::Workspace,
            HyprEvent::ActiveWindow,
            HyprEvent::OpenWindow,
            HyprEvent::CloseWindow,
            HyprEvent::MoveWindow,
        ] {
            assert_eq!(next_event(&mut reader), Some(event));
        }
        assert_eq!(next_event(&mut reader), None);
    }

    #[test]
    fn active_workspace_overrides_focus_history_and_includes_empty_workspaces() {
        let mut workspaces = Workspaces::from([(
            1,
            Workspace {
                windows: Vec::new(),
                tag: 1,
                order: 3,
                active: false,
            },
        )]);
        set_active_workspace(&mut workspaces, 1);
        assert!(workspaces[&1].active);
        assert_eq!(workspaces[&1].order, 0);
        set_active_workspace(&mut workspaces, 2);
        assert!(!workspaces[&1].active);
        assert!(workspaces[&2].active);
        assert!(workspaces[&2].windows.is_empty());
        assert_eq!(workspaces[&2].order, 0);
    }

    #[test]
    fn title_changes_and_monitor_focus_refresh_the_bars() {
        for name in ["windowtitle", "windowtitlev2"] {
            assert_eq!(HyprEvent::parse(name), Some(HyprEvent::WindowTitle));
        }
        for name in ["workspacev2", "focusedmon", "focusedmonv2"] {
            assert_eq!(HyprEvent::parse(name), Some(HyprEvent::Workspace));
        }
        assert_eq!(
            HyprEvent::parse("activewindowv2"),
            Some(HyprEvent::ActiveWindow)
        );
    }
}
