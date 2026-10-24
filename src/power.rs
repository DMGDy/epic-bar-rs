use std::{cell::RefCell, rc::Rc, time::Duration};

use gtk::{gio, glib, prelude::*, Box, Button, Image, Label, MenuButton, Orientation};

use crate::{config, panels, util, workspaces};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Lock,
    Logout,
    Suspend,
    Reboot,
    Poweroff,
}

impl Action {
    const ALL: [Self; 5] = [
        Self::Lock,
        Self::Suspend,
        Self::Logout,
        Self::Reboot,
        Self::Poweroff,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Lock => "Lock",
            Self::Logout => "Log out",
            Self::Suspend => "Suspend",
            Self::Reboot => "Restart",
            Self::Poweroff => "Shut down",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Lock => "system-lock-screen-symbolic",
            Self::Logout => "system-log-out-symbolic",
            Self::Suspend => "weather-clear-night-symbolic",
            Self::Reboot => "system-reboot-symbolic",
            Self::Poweroff => "system-shutdown-symbolic",
        }
    }

    /* Actions that end the session ask for a second click. */
    pub fn confirms(self) -> bool {
        matches!(self, Self::Logout | Self::Reboot | Self::Poweroff)
    }

    pub fn command(self, power: &config::Power) -> Option<&str> {
        match self {
            Self::Lock => power.lock.as_deref(),
            Self::Logout => power.logout.as_deref(),
            Self::Suspend => power.suspend.as_deref(),
            Self::Reboot => power.reboot.as_deref(),
            Self::Poweroff => power.poweroff.as_deref(),
        }
    }

    /* Whether the menu offers this action: everything with a command, plus
     * logout, which falls back to Hyprland's exit dispatcher. */
    pub fn available(self, power: &config::Power) -> bool {
        self == Self::Logout
            || self
                .command(power)
                .is_some_and(|command| !command.is_empty())
    }
}

/* The action waiting for its confirming second click, with its row widgets. */
type Armed = Rc<RefCell<Option<(Action, Button, Label)>>>;

fn run(action: Action, power: &config::Power) {
    match action.command(power).filter(|command| !command.is_empty()) {
        Some(command) => {
            if let Err(error) = util::spawn_shell(command) {
                eprintln!("epic-bar-rs: cannot run {command:?}: {error}");
            }
        }
        None if action == Action::Logout => {
            gio::spawn_blocking(|| {
                if let Err(error) = workspaces::request("dispatch hl.dsp.exit()") {
                    eprintln!("epic-bar-rs: cannot exit Hyprland: {error}");
                }
            });
        }
        None => {}
    }
}

pub fn widget(power: &config::Power) -> MenuButton {
    let body = panels::body("Power");
    let icon = crate::status_icon("system-shutdown-symbolic");
    let button = panels::menu(&icon, &body, "menubutton");
    button.add_css_class("bar-button");
    button.add_css_class("power-button");
    button.set_tooltip_text(Some("Lock, suspend, log out, restart or shut down"));
    let popover = button.popover().unwrap();
    // Only one action can be armed at a time; closing the menu disarms it.
    let armed: Armed = Rc::default();
    let power = Rc::new(power.clone());
    for action in Action::ALL
        .into_iter()
        .filter(|action| action.available(&power))
    {
        let row = Box::new(Orientation::Horizontal, 8);
        row.append(&Image::from_icon_name(action.icon()));
        let label = Label::new(Some(action.label()));
        row.append(&label);
        let choice = Button::builder()
            .child(&row)
            .css_classes(["popover-action", "power-action"])
            .build();
        let (armed, power, weak_popover) = (armed.clone(), power.clone(), popover.downgrade());
        choice.connect_clicked(move |choice| {
            let already = armed
                .borrow()
                .as_ref()
                .is_some_and(|(armed, _, _)| *armed == action);
            if action.confirms() && !already {
                disarm(&armed);
                label.set_text(&format!("Click again to {}", action.label().to_lowercase()));
                choice.add_css_class("confirm");
                *armed.borrow_mut() = Some((action, choice.clone(), label.clone()));
                let armed = armed.clone();
                glib::timeout_add_local_once(Duration::from_secs(4), move || {
                    if armed
                        .borrow()
                        .as_ref()
                        .is_some_and(|(armed, _, _)| *armed == action)
                    {
                        disarm(&armed);
                    }
                });
                return;
            }
            disarm(&armed);
            if let Some(popover) = weak_popover.upgrade() {
                popover.popdown();
            }
            run(action, &power);
        });
        body.append(&choice);
    }
    popover.connect_closed(move |_| disarm(&armed));
    button
}

fn disarm(armed: &Armed) {
    if let Some((action, button, label)) = armed.borrow_mut().take() {
        label.set_text(action.label());
        button.remove_css_class("confirm");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ending_actions_confirm_and_missing_commands_hide() {
        let mut power = config::Power::default();
        assert!(Action::Reboot.confirms());
        assert!(!Action::Lock.confirms());
        assert!(Action::ALL.iter().all(|action| action.available(&power)));
        power.suspend = None;
        power.logout = None;
        assert!(!Action::Suspend.available(&power));
        assert!(Action::Logout.available(&power), "logout falls back to IPC");
        power.lock = Some(String::new());
        assert!(!Action::Lock.available(&power));
        assert_eq!(Action::Reboot.command(&power), Some("systemctl reboot"));
    }
}
