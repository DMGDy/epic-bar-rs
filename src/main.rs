/*
*   Copyright (C) 2025  Dylan Dy OR Dylan-Matthew Garza
*
*   This program is free software: you can redistribute it and/or modify
*   it under the terms of the GNU General Public License as published by
*   the Free Software Foundation, either version 3 of the License, or
*   (at your option) any later version.
*
*   This program is distributed in the hope that it will be useful,
*   but WITHOUT ANY WARRANTY; without even the implied warranty of
*   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
*   GNU General Public License for more details.
*
*   You should have received a copy of the GNU General Public License
*   along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/
use std::{cell::Cell, fs, io, path::Path, rc::Rc};

use gtk::{
    gdk::Display, gio, glib, pango, prelude::*, Align, Application, Box, Button, Image, Label,
    Orientation, StyleProvider, ToggleButton,
};

mod audio;
mod bars;
mod brightness;
mod capture;
mod config;
mod gpu;
mod hub;
mod icons;
mod keyboard;
mod marquee;
mod media;
mod network;
mod notifications;
mod panels;
mod power;
mod previews;
mod selection;
mod status;
mod tasks;
mod theme;
mod tray;
mod util;
mod workspaces;

use crate::status::StatusLevel;
use crate::workspaces::{View, Workspaces};

const APP_ID: &str = "org.gtk_rs.epic_bar";
type NetworkSnapshot = Result<Vec<network::Network>, String>;

fn main() -> glib::ExitCode {
    // Read the config before GTK so a bad file fails fast with a clear error.
    let config = match config::load() {
        Ok(config) => Rc::new(config),
        Err(error) => {
            eprintln!("epic-bar-rs: {error}");
            return glib::ExitCode::FAILURE;
        }
    };
    workspaces::set_workspace_count(config.workspaces);
    let app = Application::builder().application_id(APP_ID).build();
    let initialization_failed = Rc::new(Cell::new(false));
    let startup_failed = initialization_failed.clone();
    app.connect_startup(move |app| match theme::load() {
        Ok(provider) => init_style(&provider),
        Err(error) => {
            eprintln!("epic-bar-rs: {error}");
            startup_failed.set(true);
            app.quit();
        }
    });
    let activation_failed = initialization_failed.clone();
    // Launching the bar again activates the running instance through D-Bus;
    // only the first activation builds bars.
    let built = Cell::new(false);
    app.connect_activate(move |app| {
        if activation_failed.get() || built.replace(true) {
            return;
        }
        if let Err(error) = bars::Shell::start(app, config.clone()) {
            eprintln!("epic-bar-rs: cannot initialize bars: {error}");
            activation_failed.set(true);
            app.quit();
        }
    });
    let result = app.run();
    if initialization_failed.get() {
        glib::ExitCode::FAILURE
    } else {
        result
    }
}

fn network_entry(text: &str, icon: &str, tooltip: &str) -> Box {
    let entry = Box::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["status-group", "status-network"])
        .valign(Align::Center)
        .spacing(4)
        .tooltip_text(tooltip)
        .build();
    let label = Label::builder()
        .css_name("net-label")
        .label(text)
        .ellipsize(pango::EllipsizeMode::End)
        .max_width_chars(16)
        .single_line_mode(true)
        .build();
    entry.append(&status_icon(icon));
    entry.append(&label);
    entry
}

fn update_networks(
    container: &Box,
    previous: &mut Option<NetworkSnapshot>,
    snapshot: NetworkSnapshot,
) {
    if previous.as_ref() == Some(&snapshot) {
        return;
    }
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    match &snapshot {
        Ok(networks) => {
            for network in networks {
                let entry = network_entry(network.label(), network.icon_name(), &network.tooltip());
                entry.add_css_class(match network.kind {
                    network::Kind::Wired => "wired",
                    network::Kind::Wireless => "wireless",
                    network::Kind::Offline => "offline",
                });
                container.append(&panels::network(&entry, network));
            }
        }
        Err(error) => {
            eprintln!("epic-bar-rs: cannot read network devices: {error}");
            let entry = network_entry("Network error", "dialog-warning-symbolic", error);
            entry.add_css_class("warning");
            container.append(&entry);
        }
    }
    *previous = Some(snapshot);
}

fn workspace_button(tag: usize) -> Button {
    let button = Button::builder()
        .label(tag.to_string())
        .css_classes(["bar-button", "workspace-button"])
        .name(tag.to_string())
        .valign(Align::Fill)
        .visible(true)
        .focus_on_click(true)
        .build();
    button.connect_clicked(move |_| {
        gio::spawn_blocking(move || {
            workspaces::switch_workspace(tag);
        });
    });
    button
}

fn clock_button(date: &Label, time: &Label) -> gtk::MenuButton {
    date.set_xalign(1.0);
    time.set_xalign(1.0);
    let clock = Box::builder()
        .orientation(Orientation::Vertical)
        .valign(Align::Center)
        .spacing(0)
        .build();
    clock.append(date);
    clock.append(time);
    let contents = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .build();
    contents.append(&status_icon("x-office-calendar-symbolic"));
    contents.append(&clock);
    let button = panels::calendar(&contents);
    button.set_tooltip_text(Some("Local date and time - click for calendar"));
    button
}

fn metric_label(css_name: &str) -> Label {
    let label = Label::builder()
        .css_name(css_name)
        .css_classes(["numeric"])
        .label("--")
        .ellipsize(pango::EllipsizeMode::End)
        .single_line_mode(true)
        .xalign(0.0)
        .build();
    let attributes = pango::AttrList::new();
    attributes.insert(pango::AttrFontFeatures::new("tnum=1"));
    label.set_attributes(Some(&attributes));
    label
}

fn status_icon(name: &str) -> Image {
    Image::builder()
        .icon_name(name)
        .pixel_size(18)
        .css_classes(["status-icon"])
        .build()
}

fn update_status(group: &Box, icon: &Image, level: StatusLevel, normal_icon: &str) {
    group.remove_css_class("warning");
    group.remove_css_class("critical");
    let name = match level {
        StatusLevel::Normal => normal_icon,
        StatusLevel::Warning => {
            group.add_css_class("warning");
            "dialog-warning-symbolic"
        }
        StatusLevel::Critical => {
            group.add_css_class("critical");
            "dialog-error-symbolic"
        }
    };
    icon.set_icon_name(Some(name));
}

fn connect_status_toggle(toggle: &ToggleButton, cpu: &Label, memory: &Label, detail: &Label) {
    let cpu = cpu.clone();
    let memory = memory.clone();
    let detail = detail.clone();
    toggle.connect_toggled(move |toggle| {
        let expanded = toggle.is_active();
        cpu.set_visible(expanded);
        memory.set_visible(!expanded);
        detail.set_visible(expanded);
        toggle.set_tooltip_text(Some(if expanded {
            "Hide CPU frequency and memory capacity"
        } else {
            "Show CPU frequency and memory capacity"
        }));
    });
}

fn init_style(provider: &impl IsA<StyleProvider>) {
    match Display::default() {
        Some(display) => gtk::style_context_add_provider_for_display(
            &display,
            provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        ),
        None => eprintln!("epic-bar-rs: no display to apply the theme to"),
    }
}

/* Show the buttons for existing workspaces in `view`, marking the one the
 * view's output currently shows (or the focused one without an output). */
fn populate_workspace_box(workspace_container: &Box, workspaces: &Workspaces, view: &View) {
    let mut ws_opt = workspace_container.first_child();

    while let Some(ref workspace) = ws_opt {
        let Ok(tag) = workspace.widget_name().as_str().parse::<usize>() else {
            ws_opt = workspace.next_sibling();
            continue;
        };
        let workspace_info_opt = workspaces.get(&tag).filter(|_| view.shows(tag));
        // make sure each workspace button has only one active class at a time
        if let Some(workspace_info) = workspace_info_opt {
            let active = view.is_active(workspace_info);
            workspace.set_visible(true);
            workspace.set_tooltip_text(Some(&workspace_info.tooltip()));
            if workspace.has_css_class("preview-anchor") {
                workspace.set_has_tooltip(false);
            }
            theme::set_urgent(workspace, workspace_info.urgent());
            if active && workspace_info.windows.is_empty() {
                workspace.remove_css_class("occupied");
                workspace.remove_css_class("active");
                workspace.add_css_class("empty-active");
            } else if active && !workspace_info.windows.is_empty() {
                workspace.remove_css_class("occupied");
                workspace.remove_css_class("empty-active");
                workspace.add_css_class("active");
            } else {
                workspace.remove_css_class("active");
                workspace.remove_css_class("empty-active");
                workspace.add_css_class("occupied");
            }
        } else {
            workspace.set_visible(false);
            workspace.set_tooltip_text(None);
            theme::set_urgent(workspace, false);
        }
        ws_opt = workspace.next_sibling();
    }
}

fn task_label(title: &str) -> Label {
    Label::builder()
        .label(title)
        .css_name("window-label")
        .ellipsize(pango::EllipsizeMode::End)
        .max_width_chars(24)
        .single_line_mode(true)
        .build()
}

fn task_scroller(container: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let tasks = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(container)
        .build();
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let adjustment = tasks.hadjustment();
    scroll.connect_scroll(move |_, _, dy| {
        if adjustment.upper() > adjustment.page_size() {
            adjustment.set_value(adjustment.value() + dy * 48.0);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    tasks.add_controller(scroll);
    tasks
}

fn has_battery() -> io::Result<bool> {
    let power_supply = Path::new("/sys/class/power_supply");
    if power_supply.is_dir() {
        for entry in fs::read_dir(power_supply)? {
            if entry?.file_name().to_string_lossy().contains("BAT") {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspaces::Workspace;

    fn assert_color(widget: &impl IsA<gtk::Widget>, expected: &str) {
        let widget: &gtk::Widget = widget.as_ref();
        assert_eq!(
            widget.color(),
            gtk::gdk::RGBA::parse(expected).unwrap(),
            "unexpected color for {:?}",
            widget.css_classes()
        );
    }

    #[allow(deprecated)]
    pub(crate) fn assert_attention_background(widget: &impl IsA<gtk::Widget>) {
        let widget = widget.as_ref();
        assert!(widget.has_css_class("urgent"));
        assert!(widget.has_css_class("attention-static"));
        let snapshot = gtk::Snapshot::new();
        snapshot.render_background(&widget.style_context(), 0.0, 0.0, 40.0, 40.0);
        snapshot.render_frame(&widget.style_context(), 0.0, 0.0, 40.0, 40.0);
        let node = snapshot
            .to_node()
            .expect("attention must paint a background");
        let mut surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 40, 40).unwrap();
        node.draw(&gtk::cairo::Context::new(&surface).unwrap());
        let stride = surface.stride() as usize;
        let offset = 20 * stride + 20 * 4;
        let data = surface.data().unwrap();
        assert_eq!(
            u32::from_ne_bytes(data[offset..offset + 4].try_into().unwrap()),
            0xffffe3a8,
            "attention background was hidden by focus or hover styling"
        );
        if widget.has_css_class("active") {
            let offset = 39 * stride + 20 * 4;
            assert_eq!(
                u32::from_ne_bytes(data[offset..offset + 4].try_into().unwrap()),
                0xff8c1612,
                "attention hid the red active-workspace underline"
            );
        }
    }

    // GTK 4.16 exposes CSS background rendering through StyleContext.
    #[allow(deprecated)]
    fn assert_tooltip_styles() {
        let settings = gtk::Settings::default().unwrap();
        let original = settings.property::<bool>("gtk-application-prefer-dark-theme");
        let luminance = |color: gtk::gdk::RGBA| {
            let channels = [color.red(), color.green(), color.blue()].map(|value| {
                let value = f64::from(value);
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            });
            channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722
        };
        for dark in [false, true] {
            settings.set_property("gtk-application-prefer-dark-theme", dark);
            let text = Label::new(Some("Window <debug> & title"));
            let contents = Box::new(Orientation::Horizontal, 0);
            contents.append(&text);
            let tooltip = gtk::Window::builder()
                .css_name("tooltip")
                .css_classes(["background"])
                .child(&contents)
                .build();
            assert_color(&text, "#38231e");
            let snapshot = gtk::Snapshot::new();
            snapshot.render_background(&tooltip.style_context(), 0.0, 0.0, 160.0, 80.0);
            let node = snapshot.to_node().expect("tooltip must paint a background");
            let mut surface =
                gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 160, 80).unwrap();
            {
                let context = gtk::cairo::Context::new(&surface).unwrap();
                node.draw(&context);
            }
            let offset = 40 * surface.stride() as usize + 80 * 4;
            let data = surface.data().unwrap();
            let background = u32::from_ne_bytes(data[offset..offset + 4].try_into().unwrap());
            assert_eq!(
                background, 0xfffffaf7,
                "tooltip background depends on GTK dark preference: {dark}"
            );
            let contrast = (luminance(gtk::gdk::RGBA::parse("#fffaf7").unwrap()) + 0.05)
                / (luminance(text.color()) + 0.05);
            assert!(contrast >= 7.0, "tooltip contrast is only {contrast:.1}:1");
            tooltip.destroy();
        }
        settings.set_property("gtk-application-prefer-dark-theme", original);
    }

    pub(crate) fn assert_refined_widgets() {
        let icons = gtk::IconTheme::for_display(&Display::default().unwrap());
        for name in [
            "utilities-system-monitor-symbolic",
            "drive-harddisk-symbolic",
            "view-more-symbolic",
            "x-office-calendar-symbolic",
            "network-wired-symbolic",
            "network-wireless-symbolic",
            "network-offline-symbolic",
            "dialog-warning-symbolic",
            "dialog-error-symbolic",
            "battery-missing-symbolic",
            "battery-caution-symbolic",
            "battery-low-symbolic",
            "battery-good-symbolic",
            "battery-full-symbolic",
            "battery-caution-charging-symbolic",
            "battery-low-charging-symbolic",
            "battery-good-charging-symbolic",
            "battery-full-charging-symbolic",
        ] {
            assert!(icons.has_icon(name), "missing symbolic icon: {name}");
        }

        for (name, values) in [
            ("cpu-load-label", vec!["1%", "99%", "100%"]),
            ("cpu-label", vec!["999 MHz", "5.2 GHz", "12.0 GHz"]),
            ("mem-label", vec!["1.0/8.0 GiB", "128.0/128.0 GiB"]),
            ("date-label", vec!["2026/01/11", "2026/10/20", "2026/12/31"]),
            ("date-label", vec!["01:11:11", "23:59:59"]),
        ] {
            let label = metric_label(name);
            let mut previous = None;
            for value in values {
                label.set_text(value);
                let (_, natural, _, _) = label.measure(Orientation::Horizontal, -1);
                let (_, height, _, _) = label.measure(Orientation::Vertical, natural);
                label.allocate(natural, height, -1, None);
                assert!(
                    !label.layout().is_ellipsized(),
                    "metric was clipped: {value}"
                );
                assert_eq!(
                    natural,
                    label.layout().pixel_size().0,
                    "metric has unused character slots: {value}"
                );
                assert_eq!(
                    label.layout_offsets().0,
                    0,
                    "value is separated from its icon"
                );
                if let Some(previous) = previous.filter(|_| name == "date-label") {
                    assert_eq!(natural, previous, "metric width changed for {value}");
                }
                previous = Some(natural);
            }
        }

        let networks = vec![
            network::Network {
                iface: "eth0".into(),
                kind: network::Kind::Wired,
                ssid: None,
                signal: None,
            },
            network::Network {
                iface: "wlan0".into(),
                kind: network::Kind::Wireless,
                ssid: Some("Work WiFi".into()),
                signal: None,
            },
        ];
        let area = Box::new(Orientation::Horizontal, 6);
        let mut previous = None;
        let pending = network_entry("Network...", "network-wired-symbolic", "Checking");
        pending.add_css_class("pending");
        assert_color(&pending.first_child().unwrap(), "#64748b");
        assert_color(&pending.last_child().unwrap(), "#634034");
        update_networks(&area, &mut previous, Ok(networks.clone()));
        let first = area.first_child().unwrap();
        assert!(
            first.next_sibling().is_some(),
            "both active devices must be visible"
        );
        let group = |widget: &gtk::Widget| {
            widget
                .clone()
                .downcast::<gtk::MenuButton>()
                .unwrap()
                .child()
                .unwrap()
                .downcast::<Box>()
                .unwrap()
        };
        assert_color(&group(&first).first_child().unwrap(), "#0891b2");
        assert_color(
            &group(&first.next_sibling().unwrap()).first_child().unwrap(),
            "#16a34a",
        );
        assert_color(&group(&first).last_child().unwrap(), "#634034");
        update_networks(&area, &mut previous, Ok(networks.clone()));
        assert_eq!(
            area.first_child().as_ref(),
            Some(&first),
            "unchanged devices must retain their widgets"
        );
        update_networks(&area, &mut previous, Ok(vec![network::Network::default()]));
        let offline_button = area.first_child().unwrap();
        let offline = group(&offline_button);
        assert_eq!(
            offline
                .last_child()
                .unwrap()
                .downcast::<Label>()
                .unwrap()
                .text(),
            "Offline"
        );
        assert!(
            offline_button.next_sibling().is_none(),
            "disconnected devices must disappear"
        );
        assert_color(&offline.first_child().unwrap(), "#64748b");
        assert_color(&offline.last_child().unwrap(), "#634034");
        update_networks(&area, &mut previous, Err("Network fixture error".into()));
        let error = area.first_child().unwrap();
        assert!(error.has_css_class("warning"));
        assert_color(&error.first_child().unwrap(), "#b45309");
        assert_color(&error.last_child().unwrap(), "#703900");
        update_networks(&area, &mut previous, Ok(networks));
        assert_color(
            &group(&area.first_child().unwrap()).first_child().unwrap(),
            "#0891b2",
        );

        let workspaces = Box::new(Orientation::Horizontal, 4);
        for tag in 1..=2 {
            workspaces.append(&workspace_button(tag));
        }
        let mut state = Workspaces::from([(
            1,
            Workspace {
                windows: vec![workspaces::Window {
                    name: "Ghostty".into(),
                    info: "Window fixture".into(),
                    address: "abc".into(),
                    class: "com.mitchellh.ghostty".into(),
                    tag: 1,
                    order: 0,
                    focused: true,
                    urgent: false,
                    preview: None,
                }],
                tag: 1,
                order: 0,
                active: true,
            },
        )]);
        let windows = tasks::Tasks::new(&state, None);
        populate_workspace_box(&workspaces, &state, &View::all());
        let first_workspace = workspaces.first_child().unwrap();
        assert_eq!(
            first_workspace.tooltip_text().as_deref(),
            Some("Workspace 1\n- Window fixture")
        );
        assert!(workspaces.first_child().unwrap().has_css_class("active"));
        assert!(!workspaces.last_child().unwrap().is_visible());
        assert_eq!(windows.widget().first_child().unwrap().widget_name(), "1");
        let bottom = Box::builder().css_name("bottom-bar").build();
        bottom.append(windows.widget());
        let top_label = workspaces
            .first_child()
            .unwrap()
            .downcast::<Button>()
            .unwrap()
            .child()
            .unwrap()
            .downcast::<Label>()
            .unwrap();
        let tag = windows
            .widget()
            .first_child()
            .unwrap()
            .downcast::<Label>()
            .unwrap();
        let task = tag.next_sibling().unwrap().downcast::<Button>().unwrap();
        let task_contents = task.child().unwrap().downcast::<Box>().unwrap();
        let title = task_contents
            .last_child()
            .unwrap()
            .downcast::<gtk::Overlay>()
            .unwrap()
            .child()
            .unwrap()
            .downcast::<Label>()
            .unwrap();
        let font_size = |label: &Label| label.pango_context().font_description().unwrap().size();
        assert!(font_size(&top_label) > 0);
        for label in [&tag, &title] {
            assert_eq!(
                font_size(label),
                font_size(&top_label),
                "bottom font differs from top"
            );
        }
        state.get_mut(&1).unwrap().windows[0].info = "Updated <b>title</b> & details".into();
        state.get_mut(&1).unwrap().windows.push(workspaces::Window {
            name: "Firefox".into(),
            info: "Second window".into(),
            address: "def".into(),
            class: "firefox".into(),
            tag: 1,
            order: 1,
            focused: false,
            urgent: false,
            preview: None,
        });
        populate_workspace_box(&workspaces, &state, &View::all());
        windows.update(&state);
        assert_eq!(
            first_workspace.tooltip_text().as_deref(),
            Some("Workspace 1\n- Updated <b>title</b> & details\n- Second window")
        );
        assert_eq!(
            first_workspace.tooltip_markup().as_deref(),
            Some("Workspace 1\n- Updated &lt;b&gt;title&lt;/b&gt; &amp; details\n- Second window"),
            "window titles must not be interpreted as markup"
        );
        assert_eq!(
            task.tooltip_text().as_deref(),
            Some("Updated <b>title</b> & details")
        );
        assert_eq!(title.text(), "Updated <b>title</b> & details");
        assert!(!title.uses_markup());
        let mut moved = state.get_mut(&1).unwrap().windows.remove(0);
        moved.tag = 2;
        state.insert(
            2,
            Workspace {
                windows: vec![moved],
                tag: 2,
                order: 1,
                active: false,
            },
        );
        populate_workspace_box(&workspaces, &state, &View::all());
        assert_eq!(
            first_workspace.tooltip_text().as_deref(),
            Some("Workspace 1\n- Second window")
        );
        assert_eq!(
            workspaces.last_child().unwrap().tooltip_text().as_deref(),
            Some("Workspace 2\n- Updated <b>title</b> & details")
        );
        let settings = gtk::Settings::default().unwrap();
        let animations = settings.property::<bool>("gtk-enable-animations");
        settings.set_property("gtk-enable-animations", false);
        let mut attention = workspaces::Attention::default();
        attention.observe(&workspaces::HyprEvent::Urgent("def".into()));
        attention.apply(&mut state);
        populate_workspace_box(&workspaces, &state, &View::all());
        assert!(
            first_workspace.has_css_class("active"),
            "attention changed the active workspace"
        );
        assert_attention_background(&first_workspace);
        assert_color(&top_label, "#703900");
        assert!(!workspaces.last_child().unwrap().has_css_class("urgent"));
        let selection = selection::Selection::new(&workspaces);
        assert_attention_background(&first_workspace);
        assert_eq!(
            first_workspace.tooltip_text().as_deref(),
            Some("Workspace 1\n- Second window [needs attention]")
        );
        attention.observe(&workspaces::HyprEvent::ActiveWindow(Some("def".into())));
        attention.apply(&mut state);
        populate_workspace_box(&workspaces, &state, &View::all());
        assert!(!first_workspace.has_css_class("urgent"));
        assert!(!first_workspace.has_css_class("attention-static"));
        assert_color(&top_label, "#8c1612");
        settings.set_property("gtk-enable-animations", animations);
        state.get_mut(&1).unwrap().windows[0].urgent = true;
        populate_workspace_box(&workspaces, &state, &View::all());
        assert!(first_workspace.has_css_class("urgent"));
        let empty = Workspaces::from([(
            2,
            Workspace {
                windows: Vec::new(),
                tag: 2,
                order: 0,
                active: true,
            },
        )]);
        populate_workspace_box(&workspaces, &empty, &View::all());
        windows.update(&empty);
        assert!(!workspaces.first_child().unwrap().is_visible());
        assert_eq!(first_workspace.tooltip_text(), None);
        assert!(!first_workspace.has_css_class("urgent"));
        drop(selection);
        assert!(workspaces
            .last_child()
            .unwrap()
            .has_css_class("empty-active"));
        assert_eq!(windows.widget().first_child().unwrap().widget_name(), "2");
        assert_eq!(
            workspaces.last_child().unwrap().tooltip_text().as_deref(),
            Some("Workspace 2\nNo open windows")
        );

        let cpu = metric_label("cpu-label");
        let memory = metric_label("mem-label");
        let detail = metric_label("mem-label");
        cpu.set_visible(false);
        detail.set_visible(false);
        let toggle = ToggleButton::builder()
            .css_name("status-reveal-button")
            .css_classes(["bar-button"])
            .icon_name("view-more-symbolic")
            .build();
        let more_icon = toggle.child().unwrap();
        assert_color(&more_icon, "#c2410c");
        connect_status_toggle(&toggle, &cpu, &memory, &detail);
        toggle.set_active(true);
        assert!(cpu.is_visible() && detail.is_visible() && !memory.is_visible());
        assert_color(&more_icon, "#c2410c");
        toggle.set_active(false);
        assert!(!cpu.is_visible() && !detail.is_visible() && memory.is_visible());
        assert_color(&more_icon, "#c2410c");

        for (category, normal_icon, normal_color) in [
            ("status-cpu", "utilities-system-monitor-symbolic", "#2563eb"),
            ("status-memory", "drive-harddisk-symbolic", "#9333ea"),
            ("status-battery", "battery-good-symbolic", "#15803d"),
        ] {
            let group = Box::builder()
                .css_classes(["status-group", category])
                .build();
            let icon = status_icon(normal_icon);
            let value = metric_label("mem-label");
            group.append(&icon);
            group.append(&value);
            let (_, normal_width, _, _) = group.measure(Orientation::Horizontal, -1);
            for (level, class, name, icon_color, text_color) in [
                (
                    StatusLevel::Normal,
                    "",
                    normal_icon,
                    normal_color,
                    "#634034",
                ),
                (
                    StatusLevel::Warning,
                    "warning",
                    "dialog-warning-symbolic",
                    "#b45309",
                    "#703900",
                ),
                (
                    StatusLevel::Critical,
                    "critical",
                    "dialog-error-symbolic",
                    "#dc2626",
                    "#8c1612",
                ),
                (
                    StatusLevel::Normal,
                    "",
                    normal_icon,
                    normal_color,
                    "#634034",
                ),
            ] {
                update_status(&group, &icon, level, normal_icon);
                assert_eq!(icon.icon_name().as_deref(), Some(name));
                assert_eq!(group.has_css_class("warning"), class == "warning");
                assert_eq!(group.has_css_class("critical"), class == "critical");
                assert_color(&icon, icon_color);
                assert_color(&value, text_color);
                let (_, width, _, _) = group.measure(Orientation::Horizontal, -1);
                assert_eq!(width, normal_width, "alert state changed group width");
                let (_, height, _, _) = group.measure(Orientation::Vertical, -1);
                assert!(height <= 35, "status group is too tall: {height}px");
            }
            if category == "status-battery" {
                group.add_css_class("charging");
                assert_color(&icon, "#0d9488");
                for (level, color) in [
                    (StatusLevel::Warning, "#b45309"),
                    (StatusLevel::Critical, "#dc2626"),
                    (StatusLevel::Normal, "#0d9488"),
                ] {
                    update_status(&group, &icon, level, "battery-good-charging-symbolic");
                    assert_color(&icon, color);
                }
                group.remove_css_class("charging");
                update_status(&group, &icon, StatusLevel::Normal, normal_icon);
                assert_color(&icon, normal_color);
            }
        }

        let title = task_label(&"A very long window title ".repeat(20));
        let (_, width, _, _) = title.measure(Orientation::Horizontal, -1);
        assert!(
            width <= 240,
            "task title exceeds its width budget: {width}px"
        );
        assert_eq!(title.ellipsize(), pango::EllipsizeMode::End);

        let tasks = task_scroller(&Box::new(Orientation::Horizontal, 4));
        assert_eq!(tasks.hscrollbar_policy(), gtk::PolicyType::External);
        assert_eq!(tasks.vscrollbar_policy(), gtk::PolicyType::Never);
        let controllers = tasks.observe_controllers();
        let scroll = (0..controllers.n_items())
            .filter_map(|index| controllers.item(index))
            .filter_map(|item| item.downcast::<gtk::EventControllerScroll>().ok())
            .find(|scroll| scroll.flags() == gtk::EventControllerScrollFlags::VERTICAL)
            .expect("wheel-to-horizontal-scroll controller");
        let adjustment = tasks.hadjustment();
        adjustment.configure(0.0, 0.0, 1000.0, 1.0, 100.0, 300.0);
        assert!(scroll.emit_by_name::<bool>("scroll", &[&0.0f64, &1.0f64]));
        assert_eq!(adjustment.value(), 48.0);
        assert!(scroll.emit_by_name::<bool>("scroll", &[&0.0f64, &-1.0f64]));
        assert_eq!(adjustment.value(), 0.0);
        adjustment.configure(0.0, 0.0, 300.0, 1.0, 100.0, 300.0);
        assert!(!scroll.emit_by_name::<bool>("scroll", &[&0.0f64, &1.0f64]));

        let date = metric_label("date-label");
        let time = metric_label("date-label");
        date.set_text("2026/10/20");
        time.set_text("23:59:59");
        let clock = clock_button(&date, &time);
        let clock_contents = clock.child().unwrap().downcast::<Box>().unwrap();
        let calendar = clock_contents
            .first_child()
            .unwrap()
            .downcast::<Image>()
            .unwrap();
        assert_eq!(
            calendar.icon_name().as_deref(),
            Some("x-office-calendar-symbolic")
        );
        assert_color(&calendar, "#db2777");
        assert_color(&date, "#38231e");
        assert_color(&time, "#38231e");
        let rows = clock_contents
            .last_child()
            .unwrap()
            .downcast::<Box>()
            .unwrap();
        assert_eq!(rows.orientation(), Orientation::Vertical);
        assert_eq!(rows.spacing(), 0);
        assert_eq!(rows.first_child().as_ref(), Some(date.upcast_ref()));
        assert_eq!(rows.last_child().as_ref(), Some(time.upcast_ref()));

        let ram = metric_label("mem-label");
        ram.set_text("26.9/125.1 GiB");
        let memory_group = Box::builder()
            .css_classes(["status-group"])
            .spacing(4)
            .valign(Align::Center)
            .build();
        memory_group.append(&status_icon("drive-harddisk-symbolic"));
        memory_group.append(&ram);
        let square_buttons = Box::new(Orientation::Horizontal, 4);
        for (tag, class) in [(1, "active"), (2, "occupied"), (9, "empty-active")] {
            let button = workspace_button(tag);
            button.add_css_class(class);
            square_buttons.append(&button);
        }

        for (name, expected) in [("top-bar", 48..=50), ("bottom-bar", 38..=40)] {
            let panel = Box::builder().css_name(name).build();
            let row = Box::builder()
                .css_name("main-box")
                .orientation(Orientation::Horizontal)
                .spacing(4)
                .build();
            let button = Button::builder()
                .css_classes(["bar-button"])
                .label("Workspace")
                .build();
            if name == "top-bar" {
                row.append(&square_buttons);
                row.append(&memory_group);
                row.append(&clock);
            } else {
                row.append(&button);
            }
            panel.append(&row);
            let (_, width, _, _) = panel.measure(Orientation::Horizontal, -1);
            let (_, height, _, _) = panel.measure(Orientation::Vertical, width);
            assert!(expected.contains(&height), "{name} height is {height}px");
            panel.allocate(width, height, -1, None);
            if name == "top-bar" {
                let mut child = square_buttons.first_child();
                while let Some(button) = child {
                    let bounds = button.compute_bounds(&square_buttons).unwrap();
                    assert_eq!(bounds.width(), 47.0, "workspace button width");
                    assert_eq!(bounds.height(), 47.0, "workspace button is not square");
                    let panel_bounds = button.compute_bounds(&panel).unwrap();
                    assert_eq!(panel_bounds.y(), 0.0, "highlight has a top inset");
                    assert_eq!(
                        panel_bounds.height(),
                        height as f32 - 1.0,
                        "highlight has a bottom inset beyond the separator"
                    );
                    if button == square_buttons.first_child().unwrap() {
                        assert_eq!(panel_bounds.x(), 0.0, "highlight has a left inset");
                    }
                    child = button.next_sibling();
                }
                let ram_bounds = ram.compute_bounds(&row).unwrap();
                let calendar_bounds = calendar.compute_bounds(&row).unwrap();
                let ram_right = ram_bounds.x()
                    + ram.layout_offsets().0 as f32
                    + ram.layout().pixel_size().0 as f32;
                let gap = calendar_bounds.x() - ram_right;
                assert!(
                    (8.0..=16.0).contains(&gap),
                    "RAM-to-calendar gap is {gap}px"
                );
                let ram_icon = memory_group.first_child().unwrap();
                let icon_bounds = ram_icon.compute_bounds(&row).unwrap();
                let icon_gap = ram_bounds.x() + ram.layout_offsets().0 as f32
                    - (icon_bounds.x() + icon_bounds.width());
                assert_eq!(icon_gap, 4.0, "RAM icon/value gap is too large");
                for label in [&date, &time] {
                    assert!(!label.layout().is_ellipsized(), "clock text was clipped");
                    let (_, text_height) = label.layout().pixel_size();
                    assert!(label.height() >= text_height, "clock row was too short");
                }
                assert!(
                    time.compute_bounds(&rows).unwrap().y()
                        >= date.compute_bounds(&rows).unwrap().y() + date.height() as f32,
                    "clock rows overlap"
                );
            }
        }
        crate::selection::assert_widgets();
        crate::tasks::assert_widgets();
        crate::icons::assert_widgets();
        crate::panels::assert_widgets();
        crate::media::assert_widgets();
        crate::marquee::assert_widgets();
        crate::previews::assert_widgets();
        assert_tooltip_styles();
    }
}
