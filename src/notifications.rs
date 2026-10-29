use std::{rc::Rc, sync::mpsc, thread, time::Duration};

use gtk::{glib, prelude::*, Align, Box, Button, Orientation};

use crate::{hub::Hub, util};

/* Dunst status through `dunstctl`: do-not-disturb (paused) state and how
 * many notifications are showing, queued while paused, or in history. */

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub paused: bool,
    pub displayed: u32,
    pub waiting: u32,
    pub history: u32,
}

impl State {
    /* The count worth showing: notifications held back while paused,
     * otherwise the ones on screen. */
    pub fn badge(&self) -> Option<u32> {
        let count = if self.paused {
            self.waiting
        } else {
            self.displayed
        };
        (count > 0).then_some(count)
    }

    pub fn tooltip(&self) -> String {
        let mut text = if self.paused {
            format!("Do not disturb is on - {} waiting", self.waiting)
        } else {
            format!("Notifications on - {} showing", self.displayed)
        };
        text.push_str(&format!("\n{} in history", self.history));
        text.push_str(
            "\nClick: toggle do not disturb\nRight-click: show last\nMiddle-click: dismiss all",
        );
        text
    }
}

pub type Snapshot = Result<State, String>;

fn parse_bool(text: &str) -> Result<bool, String> {
    match text.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("unexpected dunstctl is-paused reply: {other:?}")),
    }
}

fn parse_count(text: &str) -> Result<u32, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("unexpected dunstctl count reply: {:?}", text.trim()))
}

fn read() -> Snapshot {
    let count =
        |kind| util::output("dunstctl", &["count", kind]).and_then(|text| parse_count(&text));
    Ok(State {
        paused: parse_bool(&util::output("dunstctl", &["is-paused"])?)?,
        displayed: count("displayed")?,
        waiting: count("waiting")?,
        history: count("history")?,
    })
}

pub struct Notifications {
    pub hub: Rc<Hub<Snapshot>>,
    poke: mpsc::Sender<()>,
}

impl Notifications {
    /* Poll every `interval`, or immediately after the bar changes state. */
    pub fn watch(interval: Duration) -> Rc<Self> {
        let hub = Hub::new();
        let (tx, rx) = async_channel::bounded::<Snapshot>(2);
        let (poke, poked) = mpsc::channel::<()>();
        thread::spawn(move || {
            let mut previous = None;
            loop {
                let snapshot = if util::which("dunstctl").is_some() {
                    read()
                } else {
                    Err("dunstctl not found".into())
                };
                if previous.as_ref() != Some(&snapshot) {
                    if tx.send_blocking(snapshot.clone()).is_err() {
                        return;
                    }
                    previous = Some(snapshot);
                }
                match poked.recv_timeout(interval) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        let receiver = hub.clone();
        glib::spawn_future_local(async move {
            while let Ok(snapshot) = rx.recv().await {
                receiver.publish(snapshot);
            }
        });
        Rc::new(Self { hub, poke })
    }

    fn act(&self, args: &[&str]) {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let poke = self.poke.clone();
        gtk::gio::spawn_blocking(move || {
            let args: Vec<_> = args.iter().map(String::as_str).collect();
            if let Err(error) = util::output("dunstctl", &args) {
                eprintln!("epic-bar-rs: {error}");
            }
            let _ = poke.send(());
        });
    }

    pub fn widget(self: &Rc<Self>) -> Button {
        let contents = Box::builder()
            .orientation(Orientation::Horizontal)
            .css_classes(["status-group", "status-notifications"])
            .valign(Align::Center)
            .spacing(4)
            .build();
        let icon = crate::status_icon("preferences-system-notifications-symbolic");
        let count = crate::reserved_metric_label("notification-label", 4);
        contents.append(&icon);
        contents.append(&count);
        let button = Button::builder()
            .css_classes(["bar-button", "notifications-button"])
            .child(&contents)
            .visible(false)
            .build();
        let toggle = self.clone();
        button.connect_clicked(move |_| toggle.act(&["set-paused", "toggle"]));
        let history = self.clone();
        util::on_button(&button, 3, move || history.act(&["history-pop"]));
        let close = self.clone();
        util::on_button(&button, 2, move || close.act(&["close-all"]));
        let weak = button.downgrade();
        self.hub.subscribe(move |snapshot| {
            let Some(button) = weak.upgrade() else {
                return false;
            };
            let Ok(state) = snapshot else {
                button.set_visible(false);
                return true;
            };
            button.set_visible(true);
            icon.set_icon_name(Some(if state.paused {
                "notifications-disabled-symbolic"
            } else {
                "preferences-system-notifications-symbolic"
            }));
            if state.paused {
                contents.add_css_class("paused");
            } else {
                contents.remove_css_class("paused");
            }
            count.set_text(
                &state
                    .badge()
                    .map_or_else(String::new, |badge| badge.to_string()),
            );
            button.set_tooltip_text(Some(&state.tooltip()));
            true
        });
        button
    }
}

#[cfg(test)]
pub(crate) fn assert_widgets() {
    let (poke, _) = mpsc::channel();
    let notifications = Rc::new(Notifications {
        hub: Hub::new(),
        poke,
    });
    let button = notifications.widget();
    let mut previous = None;
    for count in [0, 1, 9, 10, 99, 100, 999, 0] {
        notifications.hub.publish(Ok(State {
            displayed: count,
            ..State::default()
        }));
        let contents = button.child().unwrap().downcast::<Box>().unwrap();
        let label = contents
            .last_child()
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        assert!(label.is_visible());
        assert_eq!(
            label.text(),
            if count == 0 {
                String::new()
            } else {
                count.to_string()
            }
        );
        let width = button.measure(Orientation::Horizontal, -1).1;
        if let Some(previous) = previous {
            assert_eq!(width, previous, "notification badge moved adjacent modules");
        }
        previous = Some(width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badges_show_waiting_while_paused_and_displayed_otherwise() {
        let mut state = State {
            paused: false,
            displayed: 2,
            waiting: 5,
            history: 9,
        };
        assert_eq!(state.badge(), Some(2));
        state.paused = true;
        assert_eq!(state.badge(), Some(5));
        assert!(state.tooltip().contains("5 waiting"));
        state.waiting = 0;
        assert_eq!(state.badge(), None);
        assert_eq!(parse_bool("true\n"), Ok(true));
        assert!(parse_bool("maybe").is_err());
        assert_eq!(parse_count(" 3\n"), Ok(3));
        assert!(parse_count("x").is_err());
    }
}
