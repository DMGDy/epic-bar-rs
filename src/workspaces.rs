use std::{
    io::{Write,Read,BufRead,BufReader},
    env,
    os::unix::net::UnixStream,
    iter::Peekable,
    str::SplitWhitespace,
    collections::BTreeMap,
};

const EVENTS: [&str;5] = [
            "workspace",
            "activewindow",
            "openwindow",
            "closewindow",
            "movewindow",
];

pub const WORKSPACE_COUNT: usize = 9;

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

fn peek_until_newline<'a>(iter: &mut Peekable<SplitWhitespace<'a>>,next_line: &str) -> String {
    let mut result = Vec::new();
    while let Some(&word) = iter.peek() {
        if word.contains(next_line) {
            let parts: Vec<&str> = word.split(next_line).collect();
            result.push(parts[0]); 
            break;
        }
        result.push(word);
        iter.next();
    }
    result.join(" ")
}

fn get_windows() -> Vec<Window>{
    let mut sock = get_hyprland_sock(None);

    let _ = sock.write_all(b"clients");

    let mut buff = String::new();
    sock.read_to_string(&mut buff).unwrap();


    let mut iter = buff.split_whitespace().peekable();

    let mut done = false;
    let mut all_windows: Vec<Window>  = Vec::new();
    let mut name = String::with_capacity(32);
    let mut info = String::with_capacity(64);
    let mut class = String::with_capacity(32);
    let mut address = String::with_capacity(8);
    let mut tag = usize::default();
    let mut order = usize::default();

    while let Some(key) = iter.next() {
        match key {
            "workspace:" => {
                tag = iter.peek().unwrap().parse().unwrap();
            },
            "title:" => {
                info = peek_until_newline(&mut iter,"initialClass:").trim_end().to_string();
                info = match info {
                    init_class if init_class.contains(".pdf") =>
                        init_class.split("/").last().expect("PDF Document").to_string(),
                    _ => info
                }
            },
            "initialTitle:" => {
                name = peek_until_newline(&mut iter,"pid:").trim_end().to_string();
                // special cases for ugly initialTitles
                name = match name {
                    title if title.contains("Chromium") =>  "Chromium".to_string(),
                    title if title.contains("OBS") =>  "OBS Studio".to_string(),
                    title if title.contains(".pdf") => 
                        title.split("/").last().expect("Document").to_string(),
                    title if title.contains("WhatsApp") => "WhatsApp".to_string(),
                    
                    _ => name 
                        
                }
            }
            "focusHistoryID:" => {
                order = iter.peek().unwrap().parse().unwrap();
                done = true;
            },
            
            "Window" => {
                address = iter.peek().unwrap().to_string();
            },

            "class:" => {
                class = peek_until_newline(&mut iter, "title:").trim_end().to_string();
            },

            _ => {}
        };

        if done == true{
            all_windows.push( Window{
                 name: name.clone(),
                 info: info.clone(),
                 address: address.clone(),
                 class: class.clone(),
                 tag,
                 order
            });
            done = false;
        };
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

/* read hyprland socket2 to see if there is 
 * activity on workspace or winndow change
*/ 
pub fn is_activity()  -> bool {
    let sock = get_hyprland_sock(Some("2"));

    let mut buffer = String::new();
    // for some reason reading socket2 must be buffered
    let mut reader = BufReader::new(sock);
    loop {
        match reader.read_line(&mut buffer) {
            Ok(_) =>{
                for line in buffer.lines() {
                    
                    let event = line 
                        .split(">>")
                        .next()
                        .unwrap();

                    if EVENTS.contains(&event) {
                        return true
                    }
                }
            },
            Err(_) => {}
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

    let _ = sock.write_all(b"activeworkspace");

    let mut buff = String::new();
    sock.read_to_string(&mut buff).unwrap();


    let mut iter = buff.split_whitespace().peekable();

    let mut tag  = usize::default();

    while let Some(key) = iter.next() {
        match key {
            "ID" => {
                tag = iter.peek().unwrap().parse().unwrap();
            },

            _ => {}
        };
    }

    workspaces.entry(tag)
        .or_insert( (|| { 
            let w = Workspace {
                windows: Vec::new(),
                active:  true,
                tag,
                order: 0,
            };
            w
        })());

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
