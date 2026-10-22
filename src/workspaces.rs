use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    time::Duration,
};

use serde::{de::Error as _, Deserialize};

pub const WORKSPACE_COUNT: usize = 9;

/* A bar-relevant Hyprland event read from socket2.
 * socket2 lines look like `EVENT>>data`; only events that require the bar
 * to refresh are represented here, everything else is ignored while reading.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HyprEvent {
    Workspace,
    ActiveWindow(Option<String>),
    Urgent(String),
    WindowTitle,
    OpenWindow,
    CloseWindow(String),
    MoveWindow,
}

impl HyprEvent {
    fn parse(name: &str, data: &str) -> Option<Self> {
        match name {
            "workspace" | "workspacev2" | "focusedmon" | "focusedmonv2" | "monitoradded"
            | "monitoraddedv2" | "monitorremoved" | "monitorremovedv2" | "configreloaded"
            | "activespecial" | "activespecialv2" | "moveworkspace" | "moveworkspacev2" => {
                Some(Self::Workspace)
            }
            "activewindow" => Some(Self::ActiveWindow(None)),
            "activewindowv2" if data.is_empty() => Some(Self::ActiveWindow(None)),
            "activewindowv2" | "urgent" | "closewindow" => match parse_address(data) {
                Some(address) if name == "urgent" => Some(Self::Urgent(address)),
                Some(address) if name == "closewindow" => Some(Self::CloseWindow(address)),
                Some(address) => Some(Self::ActiveWindow(Some(address))),
                None => {
                    eprintln!("epic-bar-rs: invalid {name} event address: {data:?}");
                    None
                }
            },
            "windowtitle" | "windowtitlev2" => Some(Self::WindowTitle),
            "openwindow" => Some(Self::OpenWindow),
            "movewindow" | "movewindowv2" => Some(Self::MoveWindow),
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
 * name: the initial window title, used as a fallback
 * info: the current window title (PDF paths are shown as basenames)
 * pid: pid associated with the window open
 * tag: the workspace index th window exists on
 * order: focus history, with 0 meaning most recently focused
 * focused: actual compositor focus, which can differ from focus history
 * urgent: attention observed for this window since connecting
 */
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowPreview {
    pub identifier: String,
    pub width: u32,
    pub height: u32,
    pub monitor: i64,
}

pub struct Window {
    pub name: String,
    pub info: String,
    pub address: String,
    pub class: String,
    pub tag: usize,
    pub order: usize,
    pub focused: bool,
    pub urgent: bool,
    pub preview: Option<WindowPreview>,
}

impl Window {
    pub fn caption(&self) -> &str {
        [&self.info, &self.name, &self.class]
            .into_iter()
            .map(|value| value.trim())
            .find(|value| !value.is_empty())
            .unwrap_or("Untitled window")
    }
}

fn parse_address(value: &str) -> Option<String> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    let address = usize::from_str_radix(value, 16).ok()?;
    (address != 0).then(|| format!("{address:x}"))
}

#[derive(Default)]
pub struct Attention {
    windows: BTreeSet<String>,
}

impl Attention {
    pub fn observe(&mut self, event: &HyprEvent) {
        match event {
            HyprEvent::Urgent(address) => {
                self.windows.insert(address.clone());
            }
            HyprEvent::ActiveWindow(Some(address)) | HyprEvent::CloseWindow(address) => {
                self.windows.remove(address);
            }
            _ => {}
        }
    }

    pub fn apply(&mut self, workspaces: &mut Workspaces) {
        self.windows.retain(|address| {
            workspaces.values().any(|workspace| {
                workspace
                    .windows
                    .iter()
                    .any(|window| &window.address == address && !window.focused)
            })
        });
        for workspace in workspaces.values_mut() {
            for window in &mut workspace.windows {
                window.urgent = self.windows.contains(&window.address);
            }
        }
    }
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

impl Workspace {
    pub fn urgent(&self) -> bool {
        self.windows.iter().any(|window| window.urgent)
    }

    pub fn tooltip(&self) -> String {
        let mut text = format!("Workspace {}", self.tag);
        if self.windows.is_empty() {
            text.push_str("\nNo open windows");
        }
        for window in &self.windows {
            text.push_str("\n- ");
            text.push_str(window.caption());
            if window.urgent {
                text.push_str(" [needs attention]");
            }
        }
        text
    }
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

pub(crate) fn request(command: &str) -> Result<String, String> {
    let mut socket = get_hyprland_sock(None).map_err(|error| error.to_string())?;
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .and_then(|_| socket.set_write_timeout(Some(Duration::from_secs(2))))
        .map_err(|error| error.to_string())?;
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
    #[serde(default, rename = "stableId")]
    stable_id: Option<String>,
    #[serde(default)]
    size: Option<[u32; 2]>,
    #[serde(default)]
    monitor: i64,
}

#[derive(Deserialize)]
struct JsonWorkspaceRef {
    id: i64,
}

#[derive(Deserialize)]
struct JsonActiveWorkspace {
    id: i64,
}

#[derive(Deserialize)]
struct JsonActiveWindow {
    address: Option<String>,
}

pub(crate) fn get_windows() -> Result<Vec<Window>, String> {
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

        let address = parse_address(&client.address).ok_or_else(|| {
            serde_json::Error::custom(format!("invalid window address: {:?}", client.address))
        })?;
        let preview = match (client.stable_id, client.size) {
            (Some(identifier), Some([width, height])) if width > 0 && height > 0 => {
                if parse_address(&identifier).as_deref() == Some(identifier.as_str()) {
                    Some(WindowPreview {
                        identifier,
                        width,
                        height,
                        monitor: client.monitor,
                    })
                } else {
                    eprintln!("epic-bar-rs: invalid window capture identifier: {identifier:?}");
                    None
                }
            }
            _ => None,
        };

        all_windows.push(Window {
            name,
            info,
            address,
            class: client.class,
            tag,
            // Hyprland uses -1 for clients that have not been focused yet.
            order: usize::try_from(client.focus_history_id).unwrap_or(usize::MAX),
            focused: false,
            urgent: false,
            preview,
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
                if let Some((name, data)) = line.trim_end_matches(['\r', '\n']).split_once(">>") {
                    if let Some(event) = HyprEvent::parse(name, data) {
                        return Some(event);
                    }
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
    let mut all_windows = get_windows()?;
    let reply = request("j/activewindow")?;
    let focused = parse_active_window(&reply)?;
    for window in &mut all_windows {
        window.focused = focused.as_deref() == Some(window.address.as_str());
    }
    let mut workspaces = assign_tags_to_win(all_windows);
    check_empty_active_workspace(&mut workspaces)?;

    Ok(workspaces)
}

fn parse_active_window(reply: &str) -> Result<Option<String>, String> {
    let active: JsonActiveWindow = serde_json::from_str(reply)
        .map_err(|error| format!("invalid Hyprland active window: {error}"))?;
    active
        .address
        .map(|address| {
            parse_address(&address)
                .ok_or_else(|| format!("invalid Hyprland active window address: {address:?}"))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Shutdown;

    #[test]
    fn workspace_tooltips_list_current_titles_and_handle_untitled_windows() {
        let mut workspace = Workspace {
            windows: Vec::new(),
            tag: 3,
            order: 0,
            active: true,
        };
        assert_eq!(workspace.tooltip(), "Workspace 3\nNo open windows");
        workspace.windows = parse_windows(
            r#"[
            {"address":"0xa","title":"Build <debug> & run","class":"kitty",
             "initialTitle":"Kitty","focusHistoryID":0,"workspace":{"id":3}},
            {"address":"0xb","title":"  ","class":"kitty",
             "initialTitle":"Kitty","focusHistoryID":1,"workspace":{"id":3}},
            {"address":"0xc","title":"","class":"org.editor",
             "initialTitle":"","focusHistoryID":2,"workspace":{"id":3}},
            {"address":"0xd","title":"","class":"",
             "initialTitle":"","focusHistoryID":3,"workspace":{"id":3}}
        ]"#,
        )
        .unwrap();
        assert_eq!(
            workspace
                .windows
                .iter()
                .map(Window::caption)
                .collect::<Vec<_>>(),
            [
                "Build <debug> & run",
                "Kitty",
                "org.editor",
                "Untitled window"
            ]
        );
        assert_eq!(
            workspace.tooltip(),
            "Workspace 3\n- Build <debug> & run\n- Kitty\n- org.editor\n- Untitled window"
        );
        workspace.windows[0].info = "Updated title".into();
        assert_eq!(workspace.windows[0].caption(), "Updated title");
        workspace.windows.reverse();
        assert_eq!(
            workspace.tooltip(),
            "Workspace 3\n- Untitled window\n- org.editor\n- Kitty\n- Updated title"
        );
    }

    #[test]
    fn json_clients_preserve_titles_and_handle_unfocused_or_special_windows() {
        let clients = parse_windows(
            r#"[
            {"address":"0xabc","title":"title with workspace: ID and\nnewlines","class":"kitty",
             "initialTitle":"Terminal workspace: 5","focusHistoryID":0,"workspace":{"id":1},
             "stableId":"180000af","size":[1264,662],"monitor":1},
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
        assert_eq!(
            clients[0].preview,
            Some(WindowPreview {
                identifier: "180000af".into(),
                width: 1264,
                height: 662,
                monitor: 1,
            })
        );
        assert_eq!(clients[1].preview, None);
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
            HyprEvent::ActiveWindow(None),
            HyprEvent::OpenWindow,
            HyprEvent::CloseWindow("abc".into()),
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
            assert_eq!(HyprEvent::parse(name, ""), Some(HyprEvent::WindowTitle));
        }
        for name in ["workspacev2", "focusedmon", "focusedmonv2"] {
            assert_eq!(HyprEvent::parse(name, ""), Some(HyprEvent::Workspace));
        }
        assert_eq!(
            HyprEvent::parse("activewindowv2", ""),
            Some(HyprEvent::ActiveWindow(None))
        );
    }

    #[test]
    fn urgency_events_preserve_normalized_addresses_and_focus_order() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        writer
            .write_all(b"unrelated\nurgent>>\nurgent>>invalid\nurgent>>0x00AB\r\nactivewindowv2>>ab\nurgent>>cd\nactivewindowv2>>\n")
            .unwrap();
        writer.shutdown(Shutdown::Write).unwrap();
        let mut reader = BufReader::new(reader);
        for event in [
            HyprEvent::Urgent("ab".into()),
            HyprEvent::ActiveWindow(Some("ab".into())),
            HyprEvent::Urgent("cd".into()),
            HyprEvent::ActiveWindow(None),
        ] {
            assert_eq!(next_event(&mut reader), Some(event));
        }
        assert_eq!(next_event(&mut reader), None);
        assert_eq!(HyprEvent::parse("urgent", "0"), None);
        assert_eq!(HyprEvent::parse("activewindowv2", "invalid"), None);
    }

    #[test]
    fn active_window_reply_distinguishes_no_focus_from_invalid_data() {
        assert_eq!(parse_active_window("{}").unwrap(), None);
        assert_eq!(
            parse_active_window(r#"{"address":"0x00AB","title":"Ignored"}"#).unwrap(),
            Some("ab".into())
        );
        assert!(parse_active_window("invalid").is_err());
        assert!(parse_active_window(r#"{"address":"invalid"}"#).is_err());
        let windows = parse_windows(
            r#"[{"address":"0x00AB","title":"Window","class":"kitty",
                "initialTitle":"Kitty","focusHistoryID":0,"workspace":{"id":1}}]"#,
        )
        .unwrap();
        assert_eq!(windows[0].address, "ab");
        assert!(parse_windows(
            r#"[{"address":"invalid","title":"Window","class":"kitty",
                "initialTitle":"Kitty","focusHistoryID":0,"workspace":{"id":1}}]"#,
        )
        .is_err());
    }

    #[test]
    fn attention_follows_windows_without_changing_workspace_or_window_order() {
        let mut state = assign_tags_to_win(
            parse_windows(
                r#"[
                {"address":"0xa","title":"Focused","class":"kitty",
                 "initialTitle":"Kitty","focusHistoryID":0,"workspace":{"id":1}},
                {"address":"0xb","title":"Message","class":"chat",
                 "initialTitle":"Chat","focusHistoryID":1,"workspace":{"id":1}},
                {"address":"0xc","title":"Other message","class":"chat",
                 "initialTitle":"Chat","focusHistoryID":2,"workspace":{"id":2}},
                {"address":"0xd","title":"Other window","class":"editor",
                 "initialTitle":"Editor","focusHistoryID":3,"workspace":{"id":2}}
            ]"#,
            )
            .unwrap(),
        );
        state.get_mut(&1).unwrap().windows[0].focused = true;
        let before: Vec<_> = state
            .values()
            .map(|workspace| {
                (
                    workspace.tag,
                    workspace.order,
                    workspace.active,
                    workspace
                        .windows
                        .iter()
                        .map(|w| w.address.clone())
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let mut attention = Attention::default();
        attention.observe(&HyprEvent::Urgent("b".into()));
        attention.observe(&HyprEvent::Urgent("c".into()));
        attention.observe(&HyprEvent::Urgent("b".into()));
        attention.apply(&mut state);
        assert!(state[&1].active && state[&1].urgent());
        assert!(state[&2].urgent());
        assert_eq!(
            state[&1].tooltip(),
            "Workspace 1\n- Focused\n- Message [needs attention]"
        );
        let after: Vec<_> = state
            .values()
            .map(|workspace| {
                (
                    workspace.tag,
                    workspace.order,
                    workspace.active,
                    workspace
                        .windows
                        .iter()
                        .map(|w| w.address.clone())
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(before, after);

        set_active_workspace(&mut state, 2);
        state.get_mut(&1).unwrap().windows[0].focused = false;
        state.get_mut(&2).unwrap().windows[1].focused = true;
        attention.observe(&HyprEvent::ActiveWindow(Some("d".into())));
        attention.apply(&mut state);
        assert!(
            state[&1].urgent() && state[&2].urgent(),
            "visiting a workspace cleared other windows"
        );

        let mut moved = state.get_mut(&1).unwrap().windows.remove(1);
        moved.tag = 2;
        state.get_mut(&2).unwrap().windows.push(moved);
        attention.apply(&mut state);
        assert!(!state[&1].urgent());
        assert!(state[&2].windows[2].urgent);

        // Both focus changes can be coalesced before the next snapshot.
        attention.observe(&HyprEvent::ActiveWindow(Some("c".into())));
        attention.observe(&HyprEvent::ActiveWindow(Some("d".into())));
        attention.apply(&mut state);
        assert!(!state[&2].windows[0].urgent);
        assert!(state[&2].windows[2].urgent);
        state.get_mut(&2).unwrap().windows.pop();
        attention.apply(&mut state);
        assert!(!state[&2].urgent());
        assert!(attention.windows.is_empty());
    }

    #[test]
    fn attention_reconciles_actual_focus_not_last_focus_history() {
        let mut state = assign_tags_to_win(
            parse_windows(
                r#"[{"address":"0xa","title":"Last focused","class":"kitty",
                "initialTitle":"Kitty","focusHistoryID":0,"workspace":{"id":1}}]"#,
            )
            .unwrap(),
        );
        let mut attention = Attention::default();
        attention.observe(&HyprEvent::Urgent("a".into()));
        attention.apply(&mut state);
        assert!(
            state[&1].windows[0].urgent,
            "last focused is not necessarily focused now"
        );
        state.get_mut(&1).unwrap().windows[0].focused = true;
        attention.apply(&mut state);
        assert!(!state[&1].urgent());
        assert!(attention.windows.is_empty());
        attention.observe(&HyprEvent::Urgent("dead".into()));
        attention.apply(&mut state);
        assert!(
            attention.windows.is_empty(),
            "closed windows retained attention"
        );
        state.get_mut(&1).unwrap().windows[0].focused = false;
        attention.observe(&HyprEvent::Urgent("a".into()));
        attention.observe(&HyprEvent::CloseWindow("a".into()));
        attention.observe(&HyprEvent::OpenWindow);
        attention.apply(&mut state);
        assert!(
            !state[&1].urgent(),
            "a new window reusing a closed address inherited its attention"
        );
    }
}
