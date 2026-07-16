use std::{
    io::{self, Write, Read, BufRead, BufReader},
    env,
    os::unix::net::UnixStream,
    collections::BTreeMap,
};

use serde::Deserialize;

pub const WORKSPACE_COUNT: usize = 9;

/* A bar-relevant Hyprland event read from socket2.
 * socket2 lines look like `EVENT>>data`; only events that require the bar
 * to refresh are represented here, everything else is ignored while reading.
 */
#[derive(Debug, Clone, Copy)]
pub enum HyprEvent {
    Workspace,
    ActiveWindow,
    OpenWindow,
    CloseWindow,
    MoveWindow,
}

impl HyprEvent {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "workspace" => Some(Self::Workspace),
            "activewindow" => Some(Self::ActiveWindow),
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
    pub order: usize 
}

// sorted by order (revent activity)
pub type AllWindows = Vec<Window>;

 /* windows: vector of windows in order of activity
  * order: the order of this workspace in activity
 */
pub struct Workspace {
    pub windows: Vec<Window>,
    pub tag: usize,
    pub order:  usize,
    pub active: bool 
}

pub type Workspaces = BTreeMap<usize,Workspace>;

fn get_hyprland_sock(num: Option<&str>) -> UnixStream {
    UnixStream::connect(
        format!("{}/hypr/{}/.socket{}.sock",
            env::var("XDG_RUNTIME_DIR").unwrap(),
            env::var("HYPRLAND_INSTANCE_SIGNATURE").unwrap(),
            num.unwrap_or_default()
        )).unwrap()
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
    focus_history_id: usize,
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

fn get_windows() -> Vec<Window>{
    let mut sock = get_hyprland_sock(None);

    // `j/clients` returns a JSON array of every open window
    let _ = sock.write_all(b"j/clients");

    let mut buff = String::new();
    sock.read_to_string(&mut buff).unwrap();

    let clients: Vec<JsonClient> = serde_json::from_str(&buff).unwrap_or_default();

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
            title if title.contains(".pdf") =>
                title.split("/").last().unwrap_or(&title).to_string(),
            title => title,
        };

        // name = initial title, keeping the same app-name clean-ups
        let name = match client.initial_title {
            title if title.contains("Chromium") => "Chromium".to_string(),
            title if title.contains("OBS") => "OBS Studio".to_string(),
            title if title.contains(".pdf") =>
                title.split("/").last().unwrap_or(&title).to_string(),
            title if title.contains("WhatsApp") => "WhatsApp".to_string(),
            title => title,
        };

        // JSON addresses carry a 0x prefix; strip it so switch_window
        // (which re-adds 0x) keeps working unchanged
        let address = client.address
            .strip_prefix("0x")
            .unwrap_or(&client.address)
            .to_string();

        all_windows.push(Window {
            name,
            info,
            address,
            class: client.class,
            tag,
            order: client.focus_history_id,
        });
    }

    all_windows.sort_by(|win1,win2| win1.order.cmp(&win2.order));
    all_windows
}

fn assign_tags_to_win(all_wins: AllWindows) -> Workspaces {
    let mut workspaces: Workspaces = BTreeMap::new();

    let mut order:usize = 0;
    for window in all_wins{
        workspaces.entry(window.tag)
            .or_insert( (|| { 
                let w = Workspace {
                    windows: Vec::new(),
                    active: window.order == 0,
                    tag: window.tag,
                    order,
                };
                order += 1;
                w
            })()).windows.push(window);
    }
   workspaces 
}

/* Open a connection to Hyprland's socket2 event stream.
 * The returned reader is looped over until the socket closes; the caller
 * reconnects so events are never missed for the lifetime of the bar.
 */
pub fn event_socket() -> io::Result<BufReader<UnixStream>> {
    let path = format!(
        "{}/hypr/{}/.socket2.sock",
        env::var("XDG_RUNTIME_DIR").unwrap(),
        env::var("HYPRLAND_INSTANCE_SIGNATURE").unwrap(),
    );
    Ok(BufReader::new(UnixStream::connect(path)?))
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
            Err(_) => return None,
        }
    }
}

pub fn switch_window(adr: &String) {
    
    let mut sock = get_hyprland_sock(None);
    
    let _ = sock.write_all(format!(
            "dispatch focuswindow address:0x{adr}"
    ).as_bytes());

}

pub fn switch_workspace(tag: usize) {

    let mut sock = get_hyprland_sock(None);

    let _ = sock.write_all(format!(
            "dispatch workspace {tag}"
    ).as_bytes());
   
}

fn check_empty_active_workspace(workspaces: &mut Workspaces) {
    let mut sock = get_hyprland_sock(None);

    // `j/activeworkspace` returns a JSON object for the focused workspace
    let _ = sock.write_all(b"j/activeworkspace");

    let mut buff = String::new();
    sock.read_to_string(&mut buff).unwrap();

    let active: JsonActiveWorkspace = match serde_json::from_str(&buff) {
        Ok(active) => active,
        Err(_) => return,
    };

    let tag = match usize::try_from(active.id) {
        Ok(tag) => tag,
        Err(_) => return,
    };

    workspaces.entry(tag)
        .or_insert_with(|| Workspace {
            windows: Vec::new(),
            active: true,
            tag,
            order: 0,
        });

    for (_,workspace) in workspaces {
        if workspace.tag != tag {
            workspace.order += 1;
            workspace.active = false;
        }
    }
}

pub fn get_workspaces() -> Workspaces {

    let all_windows = get_windows();
    let mut workspaces = assign_tags_to_win(all_windows);
    check_empty_active_workspace(&mut workspaces);

    workspaces
}
