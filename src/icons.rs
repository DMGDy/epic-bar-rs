use std::{collections::BTreeMap, env, path::PathBuf};

use gio_unix::DesktopAppInfo;
use gtk::{gio, prelude::*, IconTheme};

pub struct Resolver {
    apps: Vec<DesktopAppInfo>,
    cache: BTreeMap<String, gio::Icon>,
    theme: IconTheme,
    roots: Vec<PathBuf>,
}

impl Default for Resolver {
    fn default() -> Self {
        Self {
            apps: desktop_apps(),
            cache: BTreeMap::new(),
            theme: IconTheme::for_display(&gtk::gdk::Display::default().expect("GTK display")),
            roots: asset_roots(),
        }
    }
}

impl Resolver {
    pub fn refresh(&mut self) {
        self.apps = desktop_apps();
        self.cache.clear();
    }

    pub fn resolve(&mut self, class: &str) -> gio::Icon {
        if let Some(icon) = self.cache.get(class) {
            return icon.clone();
        }
        let icon = self.find(class);
        self.cache.insert(class.to_string(), icon.clone());
        icon
    }

    fn find(&self, class: &str) -> gio::Icon {
        let desktop_id = format!("{class}.desktop");
        for exact_id in [true, false] {
            for app in &self.apps {
                let matches = if exact_id {
                    app.id()
                        .is_some_and(|id| id.eq_ignore_ascii_case(&desktop_id))
                } else {
                    app.startup_wm_class()
                        .filter(|name| !name.is_empty())
                        .is_some_and(|name| name.eq_ignore_ascii_case(class))
                };
                if matches {
                    if let Some(icon) = app.icon().filter(|icon| self.available(icon)) {
                        return icon;
                    }
                }
            }
        }
        for name in [class.to_string(), class.to_ascii_lowercase()] {
            if !name.is_empty() && self.theme.has_icon(&name) {
                return gio::ThemedIcon::new(&name).upcast();
            }
        }
        if safe_class(class) {
            for root in &self.roots {
                let path = root.join(format!("{class}.svg"));
                match path.try_exists() {
                    Ok(true) => return gio::FileIcon::new(&gio::File::for_path(path)).upcast(),
                    Ok(false) => {}
                    Err(error) => eprintln!(
                        "epic-bar-rs: cannot access icon {}: {error}",
                        path.display()
                    ),
                }
            }
        }
        gio::ThemedIcon::new("application-x-executable-symbolic").upcast()
    }

    fn available(&self, icon: &gio::Icon) -> bool {
        if let Some(icon) = icon.downcast_ref::<gio::ThemedIcon>() {
            return icon.names().iter().any(|name| self.theme.has_icon(name));
        }
        if let Some(icon) = icon.downcast_ref::<gio::FileIcon>() {
            if let Some(path) = icon.file().path() {
                return match path.try_exists() {
                    Ok(exists) => exists,
                    Err(error) => {
                        eprintln!(
                            "epic-bar-rs: cannot access icon {}: {error}",
                            path.display()
                        );
                        false
                    }
                };
            }
        }
        false
    }
}

fn safe_class(class: &str) -> bool {
    !class.is_empty() && !class.contains(['/', '\\']) && class != "." && class != ".."
}

fn desktop_apps() -> Vec<DesktopAppInfo> {
    let mut apps: Vec<_> = gio::AppInfo::all()
        .into_iter()
        .filter_map(|app| app.downcast::<DesktopAppInfo>().ok())
        .collect();
    apps.sort_by_key(|app| app.id());
    apps
}

fn asset_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(config) = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    {
        roots.push(config.join("epic-bar-rs/assets/apps"));
    }
    match env::current_exe() {
        Ok(executable) => {
            if let Some(prefix) = executable.parent().and_then(std::path::Path::parent) {
                roots.push(prefix.join("share/epic-bar-rs/assets/apps"));
            }
        }
        Err(error) => eprintln!("epic-bar-rs: cannot locate installed icons: {error}"),
    }
    roots.push(PathBuf::from("assets/apps"));
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_icons_cannot_escape_their_directory() {
        for class in ["kitty", "org.mozilla.firefox", "App Name"] {
            assert!(safe_class(class));
        }
        for class in ["", ".", "..", "../outside", "/tmp/icon", r"a\b"] {
            assert!(!safe_class(class));
        }
    }
}

#[cfg(test)]
pub(crate) fn assert_widgets() {
    let mut resolver = Resolver::default();
    let key_file = gtk::glib::KeyFile::new();
    key_file.load_from_data(
        "[Desktop Entry]\nType=Application\nName=Fixture\nExec=true\nStartupWMClass=EpicBarIconFixture\nIcon=utilities-terminal-symbolic\n",
        gtk::glib::KeyFileFlags::NONE,
    ).unwrap();
    resolver.apps = vec![DesktopAppInfo::from_keyfile(&key_file).unwrap()];
    let icon = resolver.resolve("epicbariconfixture");
    assert_eq!(
        icon.downcast_ref::<gio::ThemedIcon>().unwrap().names()[0],
        "utilities-terminal-symbolic"
    );
    assert_eq!(resolver.cache.len(), 1);
    assert_eq!(resolver.resolve("epicbariconfixture"), icon);
    key_file.set_string("Desktop Entry", "StartupWMClass", "");
    resolver.apps = vec![DesktopAppInfo::from_keyfile(&key_file).unwrap()];
    let unnamed = resolver.resolve("");
    assert_eq!(
        unnamed.downcast_ref::<gio::ThemedIcon>().unwrap().names()[0],
        "application-x-executable-symbolic"
    );
    let fallback = resolver.resolve("../missing-icon");
    assert_eq!(
        fallback.downcast_ref::<gio::ThemedIcon>().unwrap().names()[0],
        "application-x-executable-symbolic"
    );
    resolver.refresh();
    assert!(resolver.cache.is_empty());
    assert!(resolver.theme.has_icon("application-x-executable-symbolic"));

    let root = env::temp_dir().join(format!("epic-bar-icons-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let desktop = root.join("EpicBarDesktopFixture.desktop");
    let bundled = root.join("EpicBarBundledFixture.svg");
    std::fs::write(&desktop, "[Desktop Entry]\nType=Application\nName=Fixture\nExec=true\nStartupWMClass=DifferentClass\nIcon=utilities-terminal-symbolic\n").unwrap();
    std::fs::write(&bundled, r#"<svg xmlns="http://www.w3.org/2000/svg" width="18" height="18"><rect width="18" height="18"/></svg>"#).unwrap();
    let app = DesktopAppInfo::from_filename(&desktop).unwrap();
    let id = app.id().unwrap();
    resolver.apps = vec![app];
    resolver.cache.clear();
    resolver.roots = vec![root.clone()];
    let icon = resolver.resolve(id.strip_suffix(".desktop").unwrap());
    assert!(
        icon.is::<gio::ThemedIcon>(),
        "desktop ID lookup did not precede bundled icons"
    );
    resolver.apps.clear();
    resolver.cache.clear();
    let icon = resolver.resolve("EpicBarBundledFixture");
    assert_eq!(
        icon.downcast_ref::<gio::FileIcon>().unwrap().file().path(),
        Some(bundled.clone())
    );
    std::fs::remove_file(desktop).unwrap();
    std::fs::remove_file(bundled).unwrap();
    std::fs::remove_dir(root).unwrap();
}
