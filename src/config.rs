use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;

/* Optional runtime configuration, read from (first match wins):
 *   1. EPIC_BAR_CONFIG, if set
 *   2. $XDG_CONFIG_HOME/epic-bar-rs/config.toml or ~/.config/epic-bar-rs/config.toml
 * A missing default file means "use the defaults"; an explicitly named file
 * that is missing, or any invalid file, is an error rather than a silent
 * fallback, matching how themes are loaded.
 */

/* Every top-bar module the bar knows how to build. Modules whose hardware or
 * service is absent (no battery, backlight, GPU, Dunst...) hide themselves. */
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Module {
    Workspaces,
    Media,
    Submap,
    Tray,
    More,
    Keyboard,
    Network,
    Cpu,
    Memory,
    Gpu,
    Battery,
    Brightness,
    Volume,
    Notifications,
    Clock,
    Power,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Bar {
    pub left: Vec<Module>,
    pub right: Vec<Module>,
    /* Show the bottom task bar. */
    pub tasks: bool,
}

impl Default for Bar {
    fn default() -> Self {
        Self {
            left: vec![Module::Workspaces, Module::Media, Module::Submap],
            right: vec![
                Module::Tray,
                Module::More,
                Module::Keyboard,
                Module::Network,
                Module::Cpu,
                Module::Memory,
                Module::Gpu,
                Module::Battery,
                Module::Brightness,
                Module::Volume,
                Module::Notifications,
                Module::Clock,
                Module::Power,
            ],
            tasks: true,
        }
    }
}

/* Per-output overrides, keyed by connector name (e.g. "DP-1", "HDMI-A-1"). */
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Output {
    /* Workspaces this output's bars show. Without it, Hyprland's workspace
     * rules decide, and outputs without rules show every unassigned one. */
    pub workspaces: Option<Vec<usize>>,
    pub left: Option<Vec<Module>>,
    pub right: Option<Vec<Module>>,
    pub tasks: Option<bool>,
    /* Do not create bars on this output at all. */
    pub disabled: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Clock {
    /* `time` crate format descriptions. */
    pub date: String,
    pub time: String,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            date: "[year]/[month]/[day]".into(),
            time: "[hour]:[minute]:[second]".into(),
        }
    }
}

/* Polling intervals in seconds. */
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Intervals {
    pub status: u64,
    pub network: u64,
    pub gpu: u64,
    pub notifications: u64,
}

impl Default for Intervals {
    fn default() -> Self {
        Self {
            status: 1,
            network: 1,
            gpu: 2,
            notifications: 2,
        }
    }
}

impl Intervals {
    pub fn duration(seconds: u64) -> Duration {
        Duration::from_secs(seconds.max(1))
    }
}

/* Shell commands for the power menu. `logout` defaults to asking Hyprland
 * to exit over IPC when unset. */
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Power {
    pub lock: Option<String>,
    pub logout: Option<String>,
    pub suspend: Option<String>,
    pub reboot: Option<String>,
    pub poweroff: Option<String>,
}

impl Default for Power {
    fn default() -> Self {
        Self {
            lock: Some("loginctl lock-session".into()),
            logout: None,
            suspend: Some("systemctl suspend".into()),
            reboot: Some("systemctl reboot".into()),
            poweroff: Some("systemctl poweroff".into()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Keyboard {
    /* Show the layout even when the keyboard has only one. */
    pub always: bool,
}

#[allow(clippy::derivable_impls)]
impl Default for Keyboard {
    fn default() -> Self {
        Self { always: false }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /* Workspace buttons 1..=workspaces. */
    pub workspaces: usize,
    pub bar: Bar,
    pub outputs: BTreeMap<String, Output>,
    pub clock: Clock,
    pub intervals: Intervals,
    pub power: Power,
    pub keyboard: Keyboard,
    /* Volume step for scrolling, in percent. */
    pub volume_step: u32,
    /* Brightness step for scrolling, in percent. */
    pub brightness_step: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            workspaces: 9,
            bar: Bar::default(),
            outputs: BTreeMap::new(),
            clock: Clock::default(),
            intervals: Intervals::default(),
            power: Power::default(),
            keyboard: Keyboard::default(),
            volume_step: 5,
            brightness_step: 5,
        }
    }
}

/* What one output's bars show, after applying its overrides. */
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub left: Vec<Module>,
    pub right: Vec<Module>,
    pub tasks: bool,
}

impl Config {
    pub fn output(&self, name: Option<&str>) -> Option<&Output> {
        name.and_then(|name| self.outputs.get(name))
    }

    pub fn layout(&self, name: Option<&str>) -> Layout {
        let output = self.output(name);
        Layout {
            left: output
                .and_then(|output| output.left.clone())
                .unwrap_or_else(|| self.bar.left.clone()),
            right: output
                .and_then(|output| output.right.clone())
                .unwrap_or_else(|| self.bar.right.clone()),
            tasks: output
                .and_then(|output| output.tasks)
                .unwrap_or(self.bar.tasks),
        }
    }

    fn validate(self) -> Result<Self, String> {
        if !(1..=99).contains(&self.workspaces) {
            return Err(format!(
                "workspaces must be between 1 and 99, not {}",
                self.workspaces
            ));
        }
        for (name, output) in &self.outputs {
            for tag in output.workspaces.iter().flatten() {
                if !(1..=self.workspaces).contains(tag) {
                    return Err(format!(
                        "outputs.\"{name}\".workspaces: {tag} is outside 1..={}",
                        self.workspaces
                    ));
                }
            }
        }
        for (name, format) in [
            ("clock.date", &self.clock.date),
            ("clock.time", &self.clock.time),
        ] {
            time::format_description::parse_borrowed::<2>(format)
                .map_err(|error| format!("{name}: invalid format {format:?}: {error}"))?;
        }
        for (name, step) in [
            ("volume_step", self.volume_step),
            ("brightness_step", self.brightness_step),
        ] {
            if !(1..=100).contains(&step) {
                return Err(format!("{name} must be between 1 and 100, not {step}"));
            }
        }
        Ok(self)
    }
}

pub fn parse(text: &str) -> Result<Config, String> {
    toml::from_str::<Config>(text)
        .map_err(|error| error.to_string())?
        .validate()
}

fn read(path: &Path) -> Result<Config, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read config {}: {error}", path.display()))?;
    parse(&text).map_err(|error| format!("invalid config {}:\n{error}", path.display()))
}

fn default_path() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|dir| dir.join("epic-bar-rs/config.toml"))
}

pub fn load() -> Result<Config, String> {
    if let Some(path) = env::var_os("EPIC_BAR_CONFIG") {
        if path.is_empty() {
            return Err("EPIC_BAR_CONFIG must name a TOML file".into());
        }
        return read(Path::new(&path));
    }
    match default_path() {
        Some(path) => match path.try_exists() {
            Ok(true) => read(&path),
            Ok(false) => Ok(Config::default()),
            Err(error) => Err(format!("cannot access config {}: {error}", path.display())),
        },
        None => Ok(Config::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_uses_defaults() {
        assert_eq!(parse("").unwrap(), Config::default());
    }

    #[test]
    fn outputs_override_modules_and_workspaces() {
        let config = parse(
            r#"
            workspaces = 10
            [bar]
            right = ["volume", "clock"]
            [outputs."HDMI-A-1"]
            workspaces = [4, 5, 6]
            right = ["clock"]
            tasks = false
            "#,
        )
        .unwrap();
        assert_eq!(config.workspaces, 10);
        assert_eq!(
            config.layout(Some("DP-1")).right,
            vec![Module::Volume, Module::Clock]
        );
        let hdmi = config.layout(Some("HDMI-A-1"));
        assert_eq!(hdmi.right, vec![Module::Clock]);
        assert!(!hdmi.tasks);
        assert_eq!(hdmi.left, Bar::default().left);
        assert_eq!(
            config.output(Some("HDMI-A-1")).unwrap().workspaces,
            Some(vec![4, 5, 6])
        );
    }

    #[test]
    fn invalid_values_are_reported() {
        assert!(parse("workspaces = 0").is_err());
        assert!(parse("workspaces = 5\n[outputs.X]\nworkspaces = [6]").is_err());
        assert!(parse("[bar]\nright = [\"nope\"]").is_err());
        assert!(parse("unknown = 1").is_err());
        assert!(parse("[clock]\ntime = \"[nope]\"").is_err());
        assert!(parse("volume_step = 0").is_err());
    }
}
