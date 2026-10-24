use std::rc::Rc;

use gtk::{gio, prelude::*, Align, Box, Button, Label, Orientation};
use serde::Deserialize;

use crate::{hub::Hub, workspaces};

/* Active keyboard layout and Hyprland submap. Both come from socket2
 * events (`activelayout`, `submap`); the initial layout from `j/devices`. */

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /* The main keyboard's active keymap, e.g. "English (US)". */
    pub keymap: String,
    /* Configured layout codes, e.g. ["us", "de"]. */
    pub layouts: Vec<String>,
    pub keyboard: String,
    /* Active submap; empty for the default bindings. */
    pub submap: String,
}

#[derive(Deserialize)]
struct JsonDevices {
    keyboards: Vec<JsonKeyboard>,
}

#[derive(Deserialize)]
struct JsonKeyboard {
    name: String,
    #[serde(default)]
    layout: String,
    #[serde(default)]
    active_keymap: String,
    #[serde(default)]
    main: bool,
}

fn parse_devices(reply: &str) -> Result<State, String> {
    let devices: JsonDevices = serde_json::from_str(reply)
        .map_err(|error| format!("invalid Hyprland devices: {error}"))?;
    let keyboard = devices
        .keyboards
        .iter()
        .find(|keyboard| keyboard.main)
        .or_else(|| devices.keyboards.first())
        .ok_or("no keyboards")?;
    Ok(State {
        keymap: keyboard.active_keymap.clone(),
        layouts: keyboard
            .layout
            .split(',')
            .map(str::trim)
            .filter(|layout| !layout.is_empty())
            .map(str::to_string)
            .collect(),
        keyboard: keyboard.name.clone(),
        submap: String::new(),
    })
}

pub fn read() -> Result<State, String> {
    parse_devices(&workspaces::request("j/devices")?)
}

impl State {
    /* Apply a socket2 event; returns whether anything visible changed. */
    pub fn observe(&mut self, event: &workspaces::HyprEvent) -> bool {
        let before = self.clone();
        match event {
            // Every keyboard reports its own layout; follow the main one,
            // or any keyboard when the main one is unknown.
            workspaces::HyprEvent::Layout(keyboard, keymap)
                if self.keyboard.is_empty() || *keyboard == self.keyboard =>
            {
                self.keymap.clone_from(keymap);
            }
            workspaces::HyprEvent::Submap(submap) => {
                self.submap = if submap == "reset" {
                    String::new()
                } else {
                    submap.clone()
                };
            }
            _ => {}
        }
        *self != before
    }

    pub fn shows_layout(&self, always: bool) -> bool {
        !self.keymap.is_empty() && (always || self.layouts.len() > 1)
    }
}

pub fn layout_widget(hub: &Rc<Hub<State>>, always: bool) -> Button {
    let contents = Box::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["status-group", "status-keyboard"])
        .valign(Align::Center)
        .spacing(4)
        .build();
    contents.append(&crate::status_icon("input-keyboard-symbolic"));
    let label = Label::builder()
        .css_name("keyboard-label")
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(14)
        .single_line_mode(true)
        .build();
    contents.append(&label);
    let button = Button::builder()
        .css_classes(["bar-button", "keyboard-button"])
        .child(&contents)
        .visible(false)
        .build();
    button.connect_clicked(|_| {
        gio::spawn_blocking(|| {
            if let Err(error) = workspaces::request("switchxkblayout all next") {
                eprintln!("epic-bar-rs: cannot switch keyboard layout: {error}");
            }
        });
    });
    let weak = button.downgrade();
    hub.subscribe(move |state| {
        let Some(button) = weak.upgrade() else {
            return false;
        };
        button.set_visible(state.shows_layout(always));
        label.set_text(&state.keymap);
        button.set_tooltip_text(Some(&format!(
            "Keyboard layout: {}\nLayouts: {}\nClick to switch",
            state.keymap,
            state.layouts.join(", ")
        )));
        true
    });
    button
}

pub fn submap_widget(hub: &Rc<Hub<State>>) -> Label {
    let label = Label::builder()
        .css_name("submap-label")
        .css_classes(["submap"])
        .visible(false)
        .valign(Align::Center)
        .build();
    let weak = label.downgrade();
    hub.subscribe(move |state| {
        let Some(label) = weak.upgrade() else {
            return false;
        };
        label.set_visible(!state.submap.is_empty());
        label.set_text(&state.submap);
        label.set_tooltip_text(Some(&format!("Submap: {}", state.submap)));
        true
    });
    label
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspaces::HyprEvent;

    #[test]
    fn layouts_follow_the_main_keyboard_and_submaps_reset() {
        let mut state = parse_devices(
            r#"{"mice":[],"keyboards":[
                {"name":"power-button","layout":"us","active_keymap":"English (US)","main":false},
                {"name":"akko","layout":"us, de","active_keymap":"English (US)","main":true}]}"#,
        )
        .unwrap();
        assert_eq!(state.keyboard, "akko");
        assert_eq!(state.layouts, ["us", "de"]);
        assert!(state.shows_layout(false));
        assert!(!state.observe(&HyprEvent::Layout("power-button".into(), "German".into())));
        assert!(state.observe(&HyprEvent::Layout("akko".into(), "German".into())));
        assert_eq!(state.keymap, "German");
        assert!(state.observe(&HyprEvent::Submap("resize".into())));
        assert_eq!(state.submap, "resize");
        assert!(state.observe(&HyprEvent::Submap(String::new())));
        assert!(state.submap.is_empty());
        state.layouts.truncate(1);
        assert!(!state.shows_layout(false));
        assert!(state.shows_layout(true));
        assert!(parse_devices(r#"{"keyboards":[]}"#).is_err());
    }
}
