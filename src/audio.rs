use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    rc::Rc,
    sync::mpsc,
    thread,
    time::Duration,
};

use gtk::{glib, prelude::*, Align, Box, Button, Label, MenuButton, Orientation, Scale};
use serde::Deserialize;

use crate::{hub::Hub, panels, util};

/* PipeWire output volume through `pactl` (pipewire-pulse). `pactl subscribe`
 * makes updates event-driven; JSON output avoids parsing localized text. */

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sink {
    pub name: String,
    pub description: String,
    /* Average channel volume in percent; may exceed 100. */
    pub volume: u32,
    pub muted: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub sinks: Vec<Sink>,
    pub default: Option<String>,
}

impl State {
    pub fn current(&self) -> Option<&Sink> {
        self.sinks
            .iter()
            .find(|sink| Some(&sink.name) == self.default.as_ref())
            .or_else(|| self.sinks.first())
    }
}

pub type Snapshot = Result<State, String>;

#[derive(Deserialize)]
struct JsonChannel {
    value: u64,
}

#[derive(Deserialize)]
struct JsonSink {
    name: String,
    #[serde(default)]
    description: String,
    mute: bool,
    volume: BTreeMap<String, JsonChannel>,
}

// PulseAudio's PA_VOLUME_NORM: 100%.
const NORM: f64 = 65536.0;

fn parse_sinks(json: &str) -> Result<Vec<Sink>, String> {
    let sinks: Vec<JsonSink> =
        serde_json::from_str(json).map_err(|error| format!("invalid pactl sinks: {error}"))?;
    Ok(sinks
        .into_iter()
        .map(|sink| {
            let channels = sink.volume.len().max(1) as f64;
            let total: f64 = sink
                .volume
                .values()
                .map(|channel| channel.value as f64)
                .sum();
            Sink {
                description: if sink.description.is_empty() {
                    sink.name.clone()
                } else {
                    sink.description
                },
                name: sink.name,
                volume: (total / channels / NORM * 100.0).round() as u32,
                muted: sink.mute,
            }
        })
        .collect())
}

fn read() -> Snapshot {
    let sinks = parse_sinks(&util::output("pactl", &["-f", "json", "list", "sinks"])?)?;
    let default = util::output("pactl", &["get-default-sink"])?
        .trim()
        .to_string();
    Ok(State {
        sinks,
        default: (!default.is_empty()).then_some(default),
    })
}

/* Whether a `pactl subscribe` line can change what the widget shows. */
fn relevant(line: &str) -> bool {
    line.contains(" sink ") || line.contains(" sink #") || line.contains(" server")
}

pub fn watch() -> Rc<Hub<Snapshot>> {
    let hub = Hub::new();
    let (tx, rx) = async_channel::bounded::<Snapshot>(4);
    thread::spawn(move || loop {
        if util::which("pactl").is_none() {
            let _ = tx.send_blocking(Err("pactl not found".into()));
            thread::sleep(Duration::from_secs(30));
            continue;
        }
        if tx.send_blocking(read()).is_err() {
            return;
        }
        let child = Command::new("pactl")
            .arg("subscribe")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                let _ = tx.send_blocking(Err(format!("cannot run pactl subscribe: {error}")));
                thread::sleep(Duration::from_secs(5));
                continue;
            }
        };
        let (poke, poked) = mpsc::channel::<()>();
        let stdout = child.stdout.take().unwrap();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if relevant(&line) && poke.send(()).is_err() {
                    break;
                }
            }
        });
        // Coalesce bursts (a volume drag emits dozens of events).
        while poked.recv().is_ok() {
            thread::sleep(Duration::from_millis(40));
            while poked.try_recv().is_ok() {}
            if tx.send_blocking(read()).is_err() {
                let _ = child.kill();
                return;
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        // The server went away (restart, logout); retry shortly.
        thread::sleep(Duration::from_secs(2));
    });
    let receiver = hub.clone();
    glib::spawn_future_local(async move {
        while let Ok(snapshot) = rx.recv().await {
            if let Err(error) = &snapshot {
                eprintln!("epic-bar-rs: volume: {error}");
            }
            receiver.publish(snapshot);
        }
    });
    hub
}

fn set_volume(percent: u32) {
    util::run_detached(
        "pactl",
        vec![
            "set-sink-volume".into(),
            "@DEFAULT_SINK@".into(),
            format!("{percent}%"),
        ],
    );
}

fn toggle_mute() {
    util::run_detached(
        "pactl",
        vec![
            "set-sink-mute".into(),
            "@DEFAULT_SINK@".into(),
            "toggle".into(),
        ],
    );
}

fn set_default(name: &str) {
    util::run_detached("pactl", vec!["set-default-sink".into(), name.into()]);
}

pub fn icon_name(sink: Option<&Sink>) -> &'static str {
    match sink {
        None => "audio-volume-muted-symbolic",
        Some(sink) if sink.muted || sink.volume == 0 => "audio-volume-muted-symbolic",
        Some(sink) if sink.volume < 34 => "audio-volume-low-symbolic",
        Some(sink) if sink.volume < 67 => "audio-volume-medium-symbolic",
        Some(_) => "audio-volume-high-symbolic",
    }
}

/* The next volume after a scroll step, clamped to 0..=100 so scrolling never
 * overdrives; values already above 100 (set elsewhere) step back down. */
pub fn stepped(volume: u32, step: u32, direction: i32) -> u32 {
    if direction < 0 {
        volume.saturating_sub(step)
    } else if volume >= 100 {
        volume
    } else {
        (volume + step).min(100)
    }
}

pub fn widget(hub: &Rc<Hub<Snapshot>>, step: u32) -> MenuButton {
    let group = Box::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["status-group", "status-volume"])
        .valign(Align::Center)
        .spacing(4)
        .build();
    let icon = crate::status_icon("audio-volume-medium-symbolic");
    let label = crate::metric_label("volume-label");
    group.append(&icon);
    group.append(&label);

    let body = panels::body("Volume");
    let scale = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    scale.set_width_request(240);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Right);
    let mute = Button::builder()
        .label("Mute")
        .css_classes(["popover-action"])
        .build();
    let outputs_heading = Label::builder()
        .label("Output")
        .css_classes(["popup-secondary"])
        .xalign(0.0)
        .build();
    let outputs = Box::new(Orientation::Vertical, 2);
    body.append(&scale);
    body.append(&mute);
    body.append(&outputs_heading);
    body.append(&outputs);
    let button = panels::menu(&group, &body, "menubutton");
    button.set_visible(false);

    let current = Rc::new(Cell::new(None::<u32>));
    let updating = Rc::new(Cell::new(false));
    // Dragging the slider would otherwise spawn pactl for every pixel.
    let pending = Rc::new(Cell::new(None::<u32>));
    let scheduled = Rc::new(Cell::new(false));
    {
        let (updating, pending, scheduled) = (updating.clone(), pending.clone(), scheduled.clone());
        scale.connect_value_changed(move |scale| {
            if updating.get() {
                return;
            }
            pending.set(Some(scale.value().round() as u32));
            if !scheduled.replace(true) {
                let (pending, scheduled) = (pending.clone(), scheduled.clone());
                glib::timeout_add_local_once(Duration::from_millis(60), move || {
                    scheduled.set(false);
                    if let Some(value) = pending.take() {
                        set_volume(value);
                    }
                });
            }
        });
    }
    mute.connect_clicked(|_| toggle_mute());
    util::on_button(&button, 3, toggle_mute);
    {
        let current = current.clone();
        util::scroll_steps(&button, move |direction| {
            if let Some(volume) = current.get() {
                set_volume(stepped(volume, step, direction));
            }
        });
    }

    let weak = button.downgrade();
    let shown_outputs = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    hub.subscribe(move |snapshot| {
        let Some(button) = weak.upgrade() else {
            return false;
        };
        let state = match snapshot {
            Ok(state) if !state.sinks.is_empty() => state,
            _ => {
                button.set_visible(false);
                current.set(None);
                return true;
            }
        };
        let sink = state.current();
        button.set_visible(true);
        icon.set_icon_name(Some(icon_name(sink)));
        if let Some(sink) = sink {
            current.set(Some(sink.volume));
            label.set_text(&if sink.muted {
                "Muted".to_string()
            } else {
                format!("{}%", sink.volume)
            });
            if sink.muted {
                group.add_css_class("muted");
            } else {
                group.remove_css_class("muted");
            }
            button.set_tooltip_text(Some(&format!(
                "{}: {}%{}\nScroll to change, right-click to mute",
                sink.description,
                sink.volume,
                if sink.muted { " (muted)" } else { "" }
            )));
            updating.set(true);
            scale.set_value(f64::from(sink.volume.min(100)));
            updating.set(false);
            mute.set_label(if sink.muted { "Unmute" } else { "Mute" });
        }
        let names: Vec<_> = state
            .sinks
            .iter()
            .map(|sink| (sink.name.clone(), sink.description.clone()))
            .collect();
        if *shown_outputs.borrow() != names || outputs.first_child().is_none() {
            while let Some(child) = outputs.first_child() {
                outputs.remove(&child);
            }
            for (name, description) in &names {
                let choice = Button::builder()
                    .label(description)
                    .css_classes(["popover-action", "audio-output"])
                    .name(name)
                    .build();
                let name = name.clone();
                choice.connect_clicked(move |_| set_default(&name));
                outputs.append(&choice);
            }
            *shown_outputs.borrow_mut() = names;
        }
        let mut child = outputs.first_child();
        while let Some(choice) = child {
            if Some(choice.widget_name().as_str()) == sink.map(|sink| sink.name.as_str()) {
                choice.add_css_class("selected");
            } else {
                choice.remove_css_class("selected");
            }
            child = choice.next_sibling();
        }
        outputs_heading.set_visible(state.sinks.len() > 1);
        outputs.set_visible(state.sinks.len() > 1);
        true
    });
    button
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sinks_average_channels_and_pick_the_default() {
        let sinks = parse_sinks(
            r#"[{"index":1,"name":"hdmi","description":"HDMI","mute":false,
                 "volume":{"front-left":{"value":32768,"value_percent":"50%","db":"x"},
                           "front-right":{"value":65536,"value_percent":"100%","db":"x"}}},
                {"index":2,"name":"usb","description":"","mute":true,
                 "volume":{"mono":{"value":0,"value_percent":"0%","db":"x"}}}]"#,
        )
        .unwrap();
        assert_eq!(sinks[0].volume, 75);
        assert_eq!(sinks[1].description, "usb");
        let state = State {
            sinks,
            default: Some("usb".into()),
        };
        assert_eq!(state.current().unwrap().name, "usb");
        assert_eq!(icon_name(state.current()), "audio-volume-muted-symbolic");
        assert_eq!(
            State {
                default: Some("gone".into()),
                ..state.clone()
            }
            .current()
            .unwrap()
            .name,
            "hdmi"
        );
        assert!(parse_sinks("nope").is_err());
    }

    #[test]
    fn scroll_steps_clamp_without_overdriving() {
        assert_eq!(stepped(50, 5, 1), 55);
        assert_eq!(stepped(98, 5, 1), 100);
        assert_eq!(stepped(120, 5, 1), 120);
        assert_eq!(stepped(120, 5, -1), 115);
        assert_eq!(stepped(3, 5, -1), 0);
    }

    #[test]
    fn subscribe_lines_filter_unrelated_events() {
        assert!(relevant("Event 'change' on sink #56"));
        assert!(relevant("Event 'change' on server #-1"));
        assert!(!relevant("Event 'change' on source-output #7"));
        assert!(!relevant("Event 'new' on client #90"));
    }
}
