use std::{cell::RefCell, rc::Rc};

use gio_unix::DesktopAppInfo;
use gtk::{gdk, gio, glib, prelude::*, Align, Box, Button, Image, Orientation};

use crate::{
    config,
    hub::Hub,
    icons, util,
    workspaces::{self, Window, Workspaces},
};

/* Pinned apps as dock-style buttons: click focuses the app's most recently
 * used window or starts it, middle-click always starts a new instance. */

/* The configured apps that are installed, in config order. */
pub type Apps = Vec<DesktopAppInfo>;

const ICON_SIZE: i32 = 22;

/* What windows are matched against for one pinned app. */
#[derive(Clone, Debug, PartialEq)]
struct Pin {
    desktop_id: String,
    wm_class: Option<String>,
}

impl Pin {
    fn new(app: &DesktopAppInfo) -> Self {
        Self {
            desktop_id: app.id().map(String::from).unwrap_or_default(),
            wm_class: app
                .startup_wm_class()
                .map(String::from)
                .filter(|name| !name.is_empty()),
        }
    }
}

/* The pinned app a window class belongs to, matched like icons::Resolver
 * picks an icon: a desktop ID equal to the class wins over a StartupWMClass
 * equal to it. */
fn owner(pins: &[Pin], class: &str) -> Option<usize> {
    if class.is_empty() {
        return None;
    }
    let desktop_id = format!("{class}.desktop");
    pins.iter()
        .position(|pin| pin.desktop_id.eq_ignore_ascii_case(&desktop_id))
        .or_else(|| {
            pins.iter().position(|pin| {
                pin.wm_class
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(class))
            })
        })
}

/* A pinned app's open windows, as far as the button cares. */
#[derive(Clone, Debug, PartialEq)]
struct Running {
    /* The most recently focused window, which a click focuses. */
    address: String,
    focused: bool,
}

/* For each pin, its running state, or None without open windows. */
fn running<'a>(pins: &[Pin], windows: impl IntoIterator<Item = &'a Window>) -> Vec<Option<Running>> {
    let mut best: Vec<Option<(usize, Running)>> = vec![None; pins.len()];
    for window in windows {
        let Some(index) = owner(pins, &window.class) else {
            continue;
        };
        match &mut best[index] {
            Some((order, app)) => {
                app.focused |= window.focused;
                if window.order < *order {
                    *order = window.order;
                    app.address.clone_from(&window.address);
                }
            }
            slot => {
                *slot = Some((
                    window.order,
                    Running {
                        address: window.address.clone(),
                        focused: window.focused,
                    },
                ))
            }
        }
    }
    best.into_iter().map(|app| app.map(|(_, app)| app)).collect()
}

/* Resolves the configured apps once for every bar, and again whenever the
 * installed apps change. */
pub struct Pinned {
    hub: Rc<Hub<Apps>>,
    _monitor: gio::AppInfoMonitor,
}

impl Pinned {
    pub fn start(config: &config::Launcher) -> Rc<Self> {
        let ids = config.desktop_ids();
        let hub = Hub::new();
        hub.publish(resolve(&ids));
        // The monitor only fires again once the app list has been re-read;
        // every launcher's icons::Resolver does that when it rebuilds.
        let monitor = gio::AppInfoMonitor::get();
        let weak = Rc::downgrade(&hub);
        monitor.connect_changed(move |_| {
            if let Some(hub) = weak.upgrade() {
                hub.publish(resolve(&ids));
            }
        });
        Rc::new(Self {
            hub,
            _monitor: monitor,
        })
    }

    pub fn hub(&self) -> &Rc<Hub<Apps>> {
        &self.hub
    }
}

fn resolve(ids: &[String]) -> Apps {
    ids.iter()
        .filter_map(|id| {
            let app = DesktopAppInfo::new(id);
            if app.is_none() {
                eprintln!("epic-bar-rs: launcher: no installed desktop entry {id}; skipping it");
            }
            app
        })
        .collect()
}

struct Entry {
    button: Button,
    pin: Pin,
    /* Window a click focuses; None launches the app instead. */
    target: Rc<RefCell<Option<String>>>,
}

impl Entry {
    fn new(app: &DesktopAppInfo, icons: &mut icons::Resolver) -> Self {
        let pin = Pin::new(app);
        let class = pin.desktop_id.strip_suffix(".desktop").unwrap_or(&pin.desktop_id);
        let icon = Image::builder()
            .gicon(&icons.resolve(class))
            .pixel_size(ICON_SIZE)
            .css_name("launcher-icon")
            .build();
        let indicator = Box::builder()
            .css_name("launcher-indicator")
            .halign(Align::Center)
            .build();
        let contents = Box::new(Orientation::Vertical, 2);
        contents.append(&icon);
        contents.append(&indicator);
        let button = Button::builder()
            .css_classes(["bar-button", "launcher-app"])
            .tooltip_text(app.display_name())
            .focus_on_click(false)
            .child(&contents)
            .build();
        let target = Rc::new(RefCell::new(None::<String>));
        let (clicked_app, clicked_target) = (app.clone(), target.clone());
        button.connect_clicked(move |_| match clicked_target.borrow().clone() {
            Some(address) => {
                gio::spawn_blocking(move || workspaces::switch_window(&address));
            }
            None => launch(&clicked_app),
        });
        let middle_app = app.clone();
        util::on_button(&button, 2, move || launch(&middle_app));
        Self { button, pin, target }
    }

    fn set(&self, running: Option<Running>) {
        let focused = running.as_ref().is_some_and(|app| app.focused);
        for (class, on) in [("running", running.is_some()), ("focused", focused)] {
            if on {
                self.button.add_css_class(class);
            } else {
                self.button.remove_css_class(class);
            }
        }
        *self.target.borrow_mut() = running.map(|app| app.address);
    }
}

#[derive(Default)]
struct State {
    entries: Vec<Entry>,
    windows: Vec<Window>,
    icons: icons::Resolver,
}

impl State {
    fn rebuild(&mut self, container: &Box, apps: &Apps) {
        while let Some(child) = container.first_child() {
            container.remove(&child);
        }
        self.icons.refresh();
        self.entries = apps
            .iter()
            .map(|app| Entry::new(app, &mut self.icons))
            .collect();
        for entry in &self.entries {
            container.append(&entry.button);
        }
        container.set_visible(!self.entries.is_empty());
        self.mark();
    }

    fn mark(&self) {
        let pins: Vec<_> = self.entries.iter().map(|entry| entry.pin.clone()).collect();
        for (entry, running) in self.entries.iter().zip(running(&pins, &self.windows)) {
            entry.set(running);
        }
    }
}

/* One bar's launcher buttons. */
pub struct Launcher {
    container: Box,
    state: Rc<RefCell<State>>,
}

impl Launcher {
    pub fn new(apps: &Rc<Hub<Apps>>) -> Self {
        let container = Box::builder()
            .orientation(Orientation::Horizontal)
            .css_name("launcher")
            .valign(Align::Center)
            .spacing(2)
            .visible(false)
            .build();
        let state = Rc::new(RefCell::new(State::default()));
        let (weak, weak_state) = (container.downgrade(), Rc::downgrade(&state));
        apps.subscribe(move |apps| {
            let (Some(container), Some(state)) = (weak.upgrade(), weak_state.upgrade()) else {
                return false;
            };
            state.borrow_mut().rebuild(&container, apps);
            true
        });
        Self { container, state }
    }

    pub fn widget(&self) -> &Box {
        &self.container
    }

    /* Mark running apps; every workspace counts, not only this output's. */
    pub fn update(&self, workspaces: &Workspaces) {
        let mut state = self.state.borrow_mut();
        state.windows = workspaces
            .values()
            .flat_map(|workspace| workspace.windows.iter().cloned())
            .collect();
        state.mark();
    }
}

/* Start a new instance of `app`. GIO spawns it detached from the bar's
 * process tree; it then gets its own systemd scope, so restarting the bar's
 * service does not take the app down with it. */
fn launch(app: &DesktopAppInfo) {
    let name = app.id().map(String::from).unwrap_or_else(|| app.name().into());
    let Some(display) = gdk::Display::default() else {
        eprintln!("epic-bar-rs: cannot launch {name}: no display");
        return;
    };
    let context = display.app_launch_context();
    let id = name.clone();
    context.connect_launched(move |_, _, platform_data| {
        match glib::VariantDict::new(Some(platform_data)).lookup::<i32>("pid") {
            Ok(Some(pid)) => match u32::try_from(pid) {
                Ok(pid) if pid > 0 => move_to_scope(&id, pid),
                _ => {}
            },
            _ => eprintln!("epic-bar-rs: launched {id} without learning its pid"),
        }
    });
    if let Err(error) = app.launch(&[], Some(&context)) {
        eprintln!("epic-bar-rs: cannot launch {name}: {error}");
    }
}

fn move_to_scope(desktop_id: &str, pid: u32) {
    let unit = scope_name(desktop_id, pid);
    glib::spawn_future_local(async move {
        let result = async {
            let bus = gio::bus_get_future(gio::BusType::Session).await?;
            let properties = vec![
                ("PIDs".to_string(), vec![pid].to_variant()),
                ("Slice".to_string(), "app.slice".to_variant()),
                ("CollectMode".to_string(), "inactive-or-failed".to_variant()),
            ];
            let auxiliary: Vec<(String, Vec<(String, glib::Variant)>)> = Vec::new();
            bus.call_future(
                Some("org.freedesktop.systemd1"),
                "/org/freedesktop/systemd1",
                "org.freedesktop.systemd1.Manager",
                "StartTransientUnit",
                Some(&(unit.as_str(), "fail", properties, auxiliary).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                2000,
            )
            .await
        }
        .await;
        if let Err(error) = result {
            eprintln!("epic-bar-rs: cannot move launched app into {unit}: {error}");
        }
    });
}

/* systemd's naming convention for launched desktop apps,
 * app-<launcher>-<app ID>-<unique>.scope, with each part escaped. */
fn scope_name(desktop_id: &str, pid: u32) -> String {
    let id = desktop_id.strip_suffix(".desktop").unwrap_or(desktop_id);
    format!("app-{}-{}-{pid}.scope", escape_unit("epic-bar"), escape_unit(id))
}

fn escape_unit(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'.') {
            escaped.push(char::from(byte));
        } else {
            escaped.push_str(&format!("\\x{byte:02x}"));
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(desktop_id: &str, wm_class: Option<&str>) -> Pin {
        Pin {
            desktop_id: desktop_id.into(),
            wm_class: wm_class.map(Into::into),
        }
    }

    fn window(class: &str, address: &str, order: usize, focused: bool) -> Window {
        Window {
            name: String::new(),
            info: String::new(),
            address: address.into(),
            class: class.into(),
            tag: 1,
            order,
            focused,
            urgent: false,
            preview: None,
        }
    }

    #[test]
    fn windows_match_desktop_ids_before_startup_wm_classes() {
        let pins = [
            pin("kitty.desktop", None),
            pin("org.mozilla.firefox.desktop", Some("firefox")),
            pin("steam.desktop", Some("Steam")),
            pin("firefox.desktop", None),
        ];
        assert_eq!(owner(&pins, "kitty"), Some(0));
        assert_eq!(owner(&pins, "Kitty"), Some(0));
        assert_eq!(owner(&pins, "steam"), Some(2));
        assert_eq!(owner(&pins, "firefox"), Some(3), "desktop ID beats StartupWMClass");
        assert_eq!(owner(&pins[..3], "firefox"), Some(1));
        assert_eq!(owner(&pins, "org.mozilla.firefox"), Some(1));
        assert_eq!(owner(&pins, "discord"), None);
        assert_eq!(owner(&[pin("x.desktop", None)], ""), None);
    }

    #[test]
    fn running_apps_target_their_most_recently_used_window() {
        let pins = [
            pin("kitty.desktop", None),
            pin("org.mozilla.firefox.desktop", Some("firefox")),
            pin("steam.desktop", None),
        ];
        let windows = [
            window("kitty", "a", 3, false),
            window("firefox", "b", 1, false),
            window("kitty", "c", 2, false),
            window("firefox", "d", 0, true),
            window("discord", "e", 4, false),
            window("kitty", "f", usize::MAX, true),
        ];
        assert_eq!(
            running(&pins, &windows),
            [
                Some(Running {
                    address: "c".into(),
                    focused: true,
                }),
                Some(Running {
                    address: "d".into(),
                    focused: true,
                }),
                None,
            ]
        );
        assert_eq!(running(&pins, &[]), [None, None, None]);
    }

    #[test]
    fn scopes_follow_systemds_app_naming() {
        assert_eq!(
            scope_name("org.mozilla.firefox.desktop", 42),
            "app-epic\\x2dbar-org.mozilla.firefox-42.scope"
        );
        assert_eq!(
            scope_name("kde-org.kde.dolphin.desktop", 7),
            "app-epic\\x2dbar-kde\\x2dorg.kde.dolphin-7.scope"
        );
        assert_eq!(escape_unit("a b/é"), "a\\x20b\\x2f\\xc3\\xa9");
    }
}

#[cfg(test)]
pub(crate) fn assert_widgets() {
    use crate::workspaces::Workspace;

    let root = std::env::temp_dir().join(format!("epic-bar-launcher-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let desktop = root.join("EpicBarLauncherFixture.desktop");
    std::fs::write(
        &desktop,
        "[Desktop Entry]\nType=Application\nName=Launcher Fixture\nExec=true\nStartupWMClass=FixtureClass\nIcon=utilities-terminal-symbolic\n",
    )
    .unwrap();
    let app = DesktopAppInfo::from_filename(&desktop).unwrap();
    std::fs::remove_file(&desktop).unwrap();
    std::fs::remove_dir(&root).unwrap();

    let hub = Hub::new();
    hub.publish(Vec::new());
    let launcher = Launcher::new(&hub);
    assert!(!launcher.widget().is_visible(), "an empty launcher must hide");
    hub.publish(vec![app.clone(), app]);
    assert!(launcher.widget().is_visible());
    let first = launcher
        .widget()
        .first_child()
        .and_downcast::<Button>()
        .unwrap();
    assert_eq!(first.tooltip_text().as_deref(), Some("Launcher Fixture"));
    assert!(first.has_css_class("bar-button"));
    assert!(!first.has_css_class("running"));

    let mut state = Workspaces::new();
    state.insert(
        1,
        Workspace {
            windows: vec![Window {
                name: "Fixture".into(),
                info: "Fixture".into(),
                address: "abc".into(),
                class: "fixtureclass".into(),
                tag: 1,
                order: 0,
                focused: false,
                urgent: false,
                preview: None,
            }],
            tag: 1,
            order: 0,
            active: true,
        },
    );
    launcher.update(&state);
    assert!(first.has_css_class("running"));
    assert!(!first.has_css_class("focused"));
    assert_eq!(
        launcher.state.borrow().entries[0].target.borrow().as_deref(),
        Some("abc")
    );
    // Rebuilding after the installed apps change keeps the running marks.
    let app = launcher.state.borrow().entries[0].button.clone();
    hub.publish(hub.with(|apps| apps.unwrap()[..1].to_vec()));
    assert_eq!(launcher.state.borrow().entries.len(), 1);
    assert_ne!(launcher.widget().first_child().as_ref(), Some(app.upcast_ref()));
    assert!(launcher
        .widget()
        .first_child()
        .unwrap()
        .has_css_class("running"));
    launcher.update(&Workspaces::new());
    assert!(!launcher
        .widget()
        .first_child()
        .unwrap()
        .has_css_class("running"));
}
