use std::{
    io::Read,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
};

use serde::Deserialize;

use crate::workspaces::{self, workspace_count, HyprEvent, WindowPreview};

pub const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
const WIDTH: f64 = 640.0;
const HEIGHT: f64 = 360.0;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    Workspace(usize),
    Window(String),
}

#[derive(Default)]
pub struct Signals {
    pub epoch: AtomicU64,
    pub popup_open: AtomicBool,
    pub connected: AtomicBool,
    changed_at: AtomicI64,
}

impl Signals {
    pub fn observe(&self, event: &HyprEvent) {
        if matches!(
            event,
            HyprEvent::Workspace
                | HyprEvent::OpenWindow
                | HyprEvent::CloseWindow(_)
                | HyprEvent::MoveWindow
        ) {
            self.invalidate();
        }
    }

    pub fn invalidate(&self) {
        self.changed_at
            .store(gtk::glib::monotonic_time(), Ordering::Release);
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }

    pub fn settled(&self) -> bool {
        gtk::glib::monotonic_time() - self.changed_at.load(Ordering::Acquire) >= 400_000
    }

    pub fn current(&self, epoch: u64) -> bool {
        self.connected.load(Ordering::Acquire) && self.epoch.load(Ordering::Acquire) == epoch
    }
}

#[derive(Clone)]
pub enum Job {
    Lock,
    Workspaces,
    Workspace(usize),
    Window(WindowPreview),
}

pub struct Image {
    pub target: Target,
    pub png: Vec<u8>,
}

pub enum Outcome {
    Locked,
    Unlocked,
    Skipped,
    Images(Vec<Image>),
}

#[derive(Deserialize)]
struct LockState {
    locked: bool,
}

pub fn locked() -> Result<bool, String> {
    let reply = workspaces::request("j/locked")?;
    serde_json::from_str::<LockState>(&reply)
        .map(|state| state.locked)
        .map_err(|error| format!("cannot read Hyprland lock state: {error}"))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct WorkspaceRef {
    id: Option<i64>,
    #[serde(default)]
    address: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct Monitor {
    id: i64,
    name: String,
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    scale: f64,
    transform: u32,
    reserved: [i32; 4],
    #[serde(rename = "activeWorkspace")]
    active: WorkspaceRef,
    #[serde(rename = "specialWorkspace")]
    special: WorkspaceRef,
    #[serde(rename = "dpmsStatus")]
    powered: bool,
}

impl Monitor {
    fn geometry(&self) -> Result<(String, f64), String> {
        if !self.scale.is_finite()
            || self.scale <= 0.0
            || self.width <= 0
            || self.height <= 0
            || self.transform > 7
            || self.reserved.iter().any(|value| *value < 0)
        {
            return Err(format!("invalid capture geometry for {}", self.name));
        }
        let (width, height) = if self.transform % 2 == 1 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        };
        let [left, top, right, bottom] = self.reserved;
        let width =
            (f64::from(width) / self.scale).floor() as i64 - i64::from(left) - i64::from(right);
        let height =
            (f64::from(height) / self.scale).floor() as i64 - i64::from(top) - i64::from(bottom);
        if width <= 0 || height <= 0 {
            return Err(format!("empty capture geometry for {}", self.name));
        }
        Ok((
            format!(
                "{},{} {}x{}",
                i64::from(self.x) + i64::from(left),
                i64::from(self.y) + i64::from(top),
                width,
                height
            ),
            scale(width as f64, height as f64),
        ))
    }

    fn workspace(&self) -> Option<usize> {
        if !self.powered
            || !self.special.address.is_empty()
            || self.special.id.is_some_and(|id| id != 0)
        {
            return None;
        }
        usize::try_from(self.active.id?)
            .ok()
            .filter(|id| (1..=workspace_count()).contains(id))
    }
}

fn monitors() -> Result<Vec<Monitor>, String> {
    // Without "all", Hyprland returns enabled outputs; some builds invert "disabled".
    let reply = workspaces::request("j/monitors")?;
    let mut monitors: Vec<Monitor> = serde_json::from_str(&reply)
        .map_err(|error| format!("cannot read capture monitors: {error}"))?;
    monitors.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(monitors)
}

fn scale(width: f64, height: f64) -> f64 {
    (WIDTH / width).min(HEIGHT / height).min(1.0)
}

fn grim_args(selector: &str, value: &str, scale: f64) -> Vec<String> {
    [
        selector.to_string(),
        value.to_string(),
        "-s".into(),
        format!("{scale:.8}"),
        "-t".into(),
        "png".into(),
        "-l".into(),
        "1".into(),
        "-".into(),
    ]
    .into()
}

fn screenshot(selector: &str, value: &str, scale: f64) -> Result<Vec<u8>, String> {
    let mut child = Command::new("timeout")
        .args(["--kill-after=0.2s", "2s", "grim"])
        .args(grim_args(selector, value, scale))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start grim capture: {error}"))?;
    let mut png = Vec::new();
    let read_png = child
        .stdout
        .take()
        .ok_or("grim capture has no output pipe")?
        .take(MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut png);
    let mut stderr = String::new();
    let read_error = child
        .stderr
        .take()
        .ok_or("grim capture has no error pipe")?
        .take(8192)
        .read_to_string(&mut stderr);
    let status = child
        .wait()
        .map_err(|error| format!("cannot wait for grim capture: {error}"))?;
    read_png.map_err(|error| format!("cannot read grim capture: {error}"))?;
    read_error.map_err(|error| format!("cannot read grim errors: {error}"))?;
    if !status.success() {
        let detail = stderr.trim().chars().take(240).collect::<String>();
        return Err(format!(
            "grim capture failed ({status}){}",
            if detail.is_empty() {
                "; check capture permission or timeout".into()
            } else {
                format!(": {detail}")
            }
        ));
    }
    png_dimensions(&png)?;
    Ok(png)
}

pub fn png_dimensions(png: &[u8]) -> Result<(u32, u32), String> {
    if png.len() > MAX_IMAGE_BYTES
        || png.len() < 33
        || &png[..8] != b"\x89PNG\r\n\x1a\n"
        || png[8..12] != 13_u32.to_be_bytes()
        || &png[12..16] != b"IHDR"
    {
        return Err("invalid or oversized preview PNG".into());
    }
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    if width == 0 || height == 0 || width > 1024 || height > 1024 {
        return Err(format!("unexpected preview dimensions: {width}x{height}"));
    }
    Ok((width, height))
}

pub fn run(job: &Job, signals: &Signals, epoch: u64) -> Result<Outcome, String> {
    if locked()? {
        return Ok(Outcome::Locked);
    }
    if !signals.current(epoch) {
        return Ok(Outcome::Skipped);
    }
    let images = match job {
        Job::Lock => return Ok(Outcome::Unlocked),
        Job::Window(wanted) => {
            let Some(source) = workspaces::get_windows()?
                .into_iter()
                .filter_map(|window| window.preview)
                .find(|source| source.identifier == wanted.identifier)
            else {
                return Ok(Outcome::Skipped);
            };
            let Some(monitor) = monitors()?
                .into_iter()
                .find(|monitor| monitor.id == source.monitor)
            else {
                return Ok(Outcome::Skipped);
            };
            if !monitor.scale.is_finite() || monitor.scale <= 0.0 {
                return Err("invalid window capture scale".into());
            }
            if !monitor.powered {
                return Ok(Outcome::Skipped);
            }
            let png = match screenshot(
                "-T",
                &source.identifier,
                scale(
                    f64::from(source.width) * monitor.scale,
                    f64::from(source.height) * monitor.scale,
                ),
            ) {
                Ok(png) => png,
                Err(error) => {
                    if locked()? {
                        return Ok(Outcome::Locked);
                    }
                    if !signals.current(epoch)
                        || !workspaces::get_windows()?.iter().any(|window| {
                            window
                                .preview
                                .as_ref()
                                .is_some_and(|live| live.identifier == source.identifier)
                        })
                    {
                        return Ok(Outcome::Skipped);
                    }
                    return Err(error);
                }
            };
            vec![Image {
                target: Target::Window(source.identifier),
                png,
            }]
        }
        Job::Workspaces | Job::Workspace(_) => {
            if signals.popup_open.load(Ordering::Acquire) {
                return Ok(Outcome::Skipped);
            }
            let before = monitors()?;
            let mut images = Vec::new();
            for monitor in &before {
                if !signals.current(epoch) || signals.popup_open.load(Ordering::Acquire) {
                    return Ok(Outcome::Skipped);
                }
                if let Some(workspace) = monitor.workspace() {
                    if matches!(job, Job::Workspace(tag) if *tag != workspace) {
                        continue;
                    }
                    let (geometry, scale) = monitor.geometry()?;
                    let png = match screenshot("-g", &geometry, scale) {
                        Ok(png) => png,
                        Err(error) => {
                            if locked()? {
                                return Ok(Outcome::Locked);
                            }
                            if !valid_batch(signals, epoch, &before, &monitors()?) {
                                return Ok(Outcome::Skipped);
                            }
                            return Err(error);
                        }
                    };
                    images.push(Image {
                        target: Target::Workspace(workspace),
                        png,
                    });
                }
            }
            if !valid_batch(signals, epoch, &before, &monitors()?) {
                return Ok(Outcome::Skipped);
            }
            images
        }
    };
    if locked()? {
        return Ok(Outcome::Locked);
    }
    Ok(if signals.current(epoch) {
        Outcome::Images(images)
    } else {
        Outcome::Skipped
    })
}

fn valid_batch(signals: &Signals, epoch: u64, before: &[Monitor], after: &[Monitor]) -> bool {
    signals.current(epoch) && !signals.popup_open.load(Ordering::Acquire) && before == after
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor() -> Monitor {
        serde_json::from_str(
            r#"{"id":1,"name":"DP-1","width":2560,"height":1440,"x":-1280,"y":0,
            "scale":2,"transform":0,"reserved":[4,48,4,38],
            "activeWorkspace":{"id":3},"specialWorkspace":{"id":0},
            "dpmsStatus":true,"disabled":true}"#,
        )
        .unwrap()
    }

    #[test]
    fn logical_geometry_handles_rotation_scale_reservations_and_negative_origins() {
        let mut monitor = monitor();
        assert_eq!(monitor.geometry().unwrap().0, "-1276,48 1272x634");
        monitor.transform = 1;
        assert_eq!(monitor.geometry().unwrap().0, "-1276,48 712x1194");
        assert_eq!(monitor.workspace(), Some(3));
        monitor.special.id = Some(-99);
        assert_eq!(monitor.workspace(), None);
        monitor.special.id = None;
        monitor.special.address = "special:chat".into();
        assert_eq!(monitor.workspace(), None);
        monitor.scale = 0.0;
        assert!(monitor.geometry().is_err());
        monitor.scale = 0.5;
        monitor.reserved[0] = 10000;
        assert!(monitor.geometry().is_err());
    }

    #[test]
    fn capture_commands_use_identifiers_and_scaled_in_memory_pngs() {
        assert_eq!(
            grim_args("-T", "180000af", scale(1280.0, 720.0)),
            [
                "-T",
                "180000af",
                "-s",
                "0.50000000",
                "-t",
                "png",
                "-l",
                "1",
                "-"
            ]
        );
        assert_eq!(scale(320.0, 180.0), 1.0);
        let mut header = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        header.extend(640_u32.to_be_bytes());
        header.extend(360_u32.to_be_bytes());
        header.resize(33, 0);
        assert_eq!(png_dimensions(&header).unwrap(), (640, 360));
        header[16..20].copy_from_slice(&100000_u32.to_be_bytes());
        assert!(png_dimensions(&header).is_err());
        assert!(png_dimensions(b"not a PNG").is_err());
        assert!(png_dimensions(&vec![0; MAX_IMAGE_BYTES + 1]).is_err());
    }

    #[test]
    fn epochs_reject_switch_away_and_back_and_preview_recursion() {
        let signals = Signals::default();
        signals.connected.store(true, Ordering::Release);
        let before = vec![monitor()];
        assert!(valid_batch(&signals, 0, &before, &before));
        signals.observe(&HyprEvent::Workspace);
        signals.observe(&HyprEvent::Workspace);
        assert!(!valid_batch(&signals, 0, &before, &before));
        assert!(valid_batch(&signals, 2, &before, &before));
        signals.popup_open.store(true, Ordering::Release);
        assert!(!valid_batch(&signals, 2, &before, &before));
        signals.popup_open.store(false, Ordering::Release);
        let mut after = before.clone();
        after[0].active.id = Some(4);
        assert!(!valid_batch(&signals, 2, &before, &after));
        signals.connected.store(false, Ordering::Release);
        assert!(!valid_batch(&signals, 2, &before, &before));
    }

    #[test]
    #[ignore = "requires a live, unlocked Hyprland session and grim 1.5+"]
    fn live_capture_backend() {
        let signals = Signals::default();
        signals.connected.store(true, Ordering::Release);
        assert!(!locked().unwrap(), "do not capture a locked session");
        let start = std::time::Instant::now();
        let Outcome::Images(images) = run(&Job::Workspaces, &signals, 0).unwrap() else {
            panic!("no workspace screenshots");
        };
        assert!(
            !images.is_empty(),
            "a powered monitor with workspace 1-9 is required"
        );
        let Target::Workspace(tag) = images[0].target else {
            panic!("workspace capture must return a workspace target");
        };
        for image in images {
            let (width, height) = png_dimensions(&image.png).unwrap();
            println!("Workspace PNG: {width}x{height}");
        }
        println!("Workspace batch: {:?}", start.elapsed());
        let start = std::time::Instant::now();
        let Outcome::Images(images) = run(&Job::Workspace(tag), &signals, 0).unwrap() else {
            panic!("no priority workspace screenshot");
        };
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].target, Target::Workspace(tag));
        png_dimensions(&images[0].png).unwrap();
        println!("Priority workspace {tag}: {:?}", start.elapsed());
        let workspaces = workspaces::get_workspaces().unwrap();
        let visible: Vec<_> = monitors()
            .unwrap()
            .iter()
            .filter_map(Monitor::workspace)
            .collect();
        let inactive = workspaces
            .values()
            .filter(|workspace| !visible.contains(&workspace.tag))
            .flat_map(|workspace| &workspace.windows)
            .find_map(|window| window.preview.clone());
        let on_inactive_workspace = inactive.is_some();
        let source = inactive
            .or_else(|| {
                workspaces
                    .values()
                    .flat_map(|workspace| &workspace.windows)
                    .find_map(|window| window.preview.clone())
            })
            .expect("a window with stable capture metadata");
        let start = std::time::Instant::now();
        let Outcome::Images(images) = run(&Job::Window(source), &signals, 0).unwrap() else {
            panic!("no window screenshot");
        };
        assert_eq!(images.len(), 1);
        let (width, height) = png_dimensions(&images[0].png).unwrap();
        println!(
            "Window PNG: {width}x{height}; {:?}; inactive workspace: {on_inactive_workspace}",
            start.elapsed()
        );
    }
}
