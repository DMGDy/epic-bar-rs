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
use std::{cell::Cell, fs, io, path::Path, rc::Rc, sync::mpsc, thread, time::Duration};

use gtk::{
    gdk::Display, gio, glib, glib::ControlFlow, pango, prelude::*, Align, Application,
    ApplicationWindow, Box, Button, Image, Label, Orientation, StyleProvider, ToggleButton,
};

use gtk4_layer_shell::{Edge, Layer, LayerShell};

mod status;
mod theme;
mod workspaces;

use crate::status::{Cpu, StatusLevel};

const APP_ID: &str = "org.gtk_rs.epic_bar";
// This cannot keep going
fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    let theme_failed = Rc::new(Cell::new(false));
    let startup_failed = theme_failed.clone();
    app.connect_startup(move |app| match theme::load() {
        Ok(provider) => init_style(&provider),
        Err(error) => {
            eprintln!("epic-bar-rs: {error}");
            startup_failed.set(true);
            app.quit();
        }
    });
    let activation_failed = theme_failed.clone();
    app.connect_activate(move |app| {
        if !activation_failed.get() {
            top_bar(app);
            bottom_bar(app);
        }
    });
    let result = app.run();
    if theme_failed.get() {
        glib::ExitCode::FAILURE
    } else {
        result
    }
}

fn top_bar(app: &Application) {
    let main_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::BaselineFill)
        .hexpand(true)
        .hexpand_set(true)
        .homogeneous(false)
        .spacing(12)
        .css_name("main-box")
        .build();

    let workspace_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .css_name("workspaces-container")
        .build();

    let spacer = Box::builder()
        .orientation(Orientation::Horizontal)
        .hexpand(true)
        .build();

    let status_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .hexpand(false)
        .spacing(6)
        .build();

    let status_reveal_button = ToggleButton::builder()
        .css_name("status-reveal-button")
        .css_classes(["bar-button"])
        .icon_name("view-more-symbolic")
        .tooltip_text("Show CPU frequency and memory capacity")
        .build();
    let has = has_battery().unwrap_or_else(|error| {
        eprintln!("epic-bar-rs: cannot detect battery: {error}");
        false
    });

    let battery_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["status-group"])
        .hexpand(false)
        .valign(Align::Center)
        .spacing(4)
        .build();

    let battery_image = status_icon("battery-missing-symbolic");
    let battery_label = metric_label("battery-label", 5);

    let mem_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["status-group"])
        .hexpand(false)
        .valign(Align::Center)
        .spacing(4)
        .build();

    let mem_icon = status_icon("drive-harddisk-symbolic");
    let mem_label = metric_label("mem-label", 5);
    let mem_detail_label = metric_label("mem-label", 17);
    mem_detail_label.set_visible(false);

    let cpu_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["status-group"])
        .hexpand(false)
        .valign(Align::Center)
        .spacing(4)
        .build();

    let cpu_image = status_icon("utilities-system-monitor-symbolic");
    let cpu_label = metric_label("cpu-label", 8);
    cpu_label.set_visible(false);
    let cpu_load_label = metric_label("cpu-load-label", 5);

    let date_container = Button::builder()
        .css_name("date-container")
        .css_classes(["bar-button"])
        .build();

    let date_label = metric_label("date-label", 8);
    date_label.set_text("00:00:00");

    cpu_container.append(&cpu_image);
    cpu_container.append(&cpu_load_label);
    cpu_container.append(&cpu_label);

    mem_container.append(&mem_icon);
    mem_container.append(&mem_label);
    mem_container.append(&mem_detail_label);

    if has {
        battery_container.append(&battery_image);
        battery_container.append(&battery_label);
    }

    date_container.set_child(Some(&date_label));

    status_container.append(&status_reveal_button);
    status_container.append(&cpu_container);
    status_container.append(&mem_container);
    if has {
        status_container.append(&battery_container);
    }

    connect_status_toggle(
        &status_reveal_button,
        &cpu_label,
        &mem_label,
        &mem_detail_label,
    );

    // init the container
    for n in 1..workspaces::WORKSPACE_COUNT + 1 {
        let workspace_button = Button::builder()
            .label(format!("{}", n))
            .css_classes(["bar-button"])
            .name(format!("{}", n))
            .visible(true)
            .focus_on_click(true)
            .build();

        let nclone = n.clone();

        workspace_button.connect_clicked(move |_| {
            gio::spawn_blocking(move || {
                workspaces::switch_workspace(nclone);
            });
        });
        workspace_container.append(&workspace_button);
    }

    populate_workspace_box(&workspace_container);

    main_container.append(&workspace_container);
    main_container.append(&spacer);
    main_container.append(&status_container);
    main_container.append(&date_container);

    // create window and set title
    let window = ApplicationWindow::builder()
        .application(app)
        .css_name("top-bar")
        .child(&main_container)
        .build();

    LayerShell::init_layer_shell(&window);
    LayerShell::set_layer(&window, Layer::Top);

    LayerShell::auto_exclusive_zone_enable(&window);

    LayerShell::set_anchor(&window, Edge::Top, true);
    LayerShell::set_anchor(&window, Edge::Left, true);
    LayerShell::set_anchor(&window, Edge::Right, true);

    window.set_decorated(true);
    window.present();

    let (tx, rx) = mpsc::channel();

    let workspace_clone = workspace_container.clone();
    // check if workspace activity in different thread to avoid blocking
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(50));
        if workspaces::is_activity() {
            tx.send(()).unwrap();
        }
    });

    // on main thread check if signal recieved that there is to update

    glib::source::timeout_add_local(Duration::from_millis(50), move || {
        if let Ok(_) = rx.try_recv() {
            populate_workspace_box(&workspace_clone);
        }

        ControlFlow::Continue
    });

    // persisting data to track cpu load over time
    let cpu = status::Cpu::new();
    let mut cpu = cpu.clone();

    // update other stuff less frequently
    glib::source::timeout_add_seconds_local(1, move || {
        if has {
            let battery = status::get_battery_info();
            battery_label.set_text(&format!("{}%", battery.capacity));
            update_status(
                &battery_container,
                &battery_image,
                battery.level(),
                &battery.icon,
            );
            battery_container.set_tooltip_text(Some(&battery.tooltip_text));
        }

        let dt = status::get_datetime();
        date_label.set_text(&dt.time);
        date_container.set_tooltip_text(Some(&dt.date));

        let memory = status::get_mem_info();
        mem_label.set_text(&format!("{:.0}%", memory.used_percent));
        mem_detail_label.set_text(&memory.string);
        mem_container.set_tooltip_text(Some(&format!("Memory: {}", memory.string)));
        update_status(
            &mem_container,
            &mem_icon,
            StatusLevel::memory(memory.used_percent),
            "drive-harddisk-symbolic",
        );

        if cpu_label.is_visible() {
            cpu_label.set_text(&Cpu::get_avg_freq());
        }
        let cpu_load = cpu.get_cpu_load();
        cpu_load_label.set_text(&format!("{:.0}%", cpu_load));
        cpu_container.set_tooltip_text(Some(&format!("CPU usage: {cpu_load:.1}%")));
        update_status(
            &cpu_container,
            &cpu_image,
            StatusLevel::cpu(cpu_load),
            "utilities-system-monitor-symbolic",
        );

        ControlFlow::Continue
    });
}

fn metric_label(css_name: &str, width: i32) -> Label {
    let label = Label::builder()
        .css_name(css_name)
        .css_classes(["numeric"])
        .label("--")
        .width_chars(width)
        .max_width_chars(width)
        .ellipsize(pango::EllipsizeMode::End)
        .single_line_mode(true)
        .xalign(1.0)
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
    let display = Display::default();
    gtk::style_context_add_provider_for_display(
        &display.unwrap(),
        provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn populate_workspace_box(workspace_container: &Box) {
    let workspaces = workspaces::get_workspaces();
    let mut ws_opt = workspace_container.first_child();

    while let Some(ref workspace) = ws_opt {
        let tag: usize = workspace.widget_name().as_str().parse().unwrap();
        let workspace_info_opt = workspaces.get(&tag);
        // make sure each workspace button has only one active class at a time
        if let Some(workspace_info) = workspace_info_opt {
            workspace.set_visible(true);
            if workspace_info.active && workspace_info.windows.is_empty() {
                workspace.remove_css_class("occupied");
                workspace.remove_css_class("active");
                workspace.add_css_class("empty-active");
            } else if workspace_info.active && !workspace_info.windows.is_empty() {
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
        }
        ws_opt = workspace.next_sibling();
    }
}

fn bottom_bar(app: &Application) {
    let main_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::BaselineFill)
        .hexpand(true)
        .hexpand_set(true)
        .homogeneous(false)
        .css_name("main-box")
        .build();

    let workspace_windows_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .hexpand(true)
        .vexpand(true)
        .homogeneous(false)
        .spacing(12)
        .css_name("workspace-windows-container")
        .build();

    populate_windows_container(&workspace_windows_container);
    let tasks = task_scroller(&workspace_windows_container);
    main_container.append(&tasks);
    let window = ApplicationWindow::builder()
        .css_name("bottom-bar")
        .resizable(true)
        .application(app)
        .child(&main_container)
        .build();
    LayerShell::init_layer_shell(&window);
    LayerShell::set_layer(&window, Layer::Top);

    LayerShell::auto_exclusive_zone_enable(&window);

    LayerShell::set_anchor(&window, Edge::Bottom, true);
    LayerShell::set_anchor(&window, Edge::Right, true);
    LayerShell::set_anchor(&window, Edge::Left, true);

    window.set_decorated(true);
    window.present();
    let (tx, rx) = mpsc::channel();

    // check if workspace activity in different thread to avoid blocking
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(25));
        if workspaces::is_activity() {
            tx.send(()).unwrap();
        }
    });

    glib::source::timeout_add_local(Duration::from_millis(25), move || {
        if let Ok(_) = rx.try_recv() {
            populate_windows_container(&workspace_windows_container);
        }

        ControlFlow::Continue
    });
}

fn populate_windows_container(container: &Box) {
    let workspaces = workspaces::get_workspaces();

    let sorted: &mut Vec<_> = &mut workspaces.into_iter().collect();

    // sort by recently used
    sorted.sort_by(|w1, w2| w1.1.order.cmp(&w2.1.order));

    let mut tag_opt = container.first_child();

    while let Some(tag) = tag_opt {
        // empty container with everything
        container.remove(&tag);
        tag_opt = container.first_child()
    }

    // start filling with occupied workspaces
    for (tag, workspace) in sorted {
        // Box will contain
        //  - label of tag
        //  - button for each window
        //      - child is box has icon
        //          - Initial_title of application

        let workspace_windows_css = if workspace.active && !workspace.windows.is_empty() {
            "workspace-window-box-active"
        } else if workspace.windows.is_empty() {
            "workspace-window-box-empty"
        } else {
            "workspace-window-box"
        };

        let workspace_box = Box::builder()
            .name(format!("{}", tag))
            .css_name(workspace_windows_css)
            .vexpand(true)
            .spacing(4)
            .build();

        let tag_label = Label::builder()
            .label(format!("{}", tag))
            .css_name("tag-label")
            .build();

        workspace_box.append(&tag_label);

        if workspace.windows.is_empty() {
            let window_button = Button::builder()
                .css_name("window-box-empty")
                .css_classes(["bar-button"])
                .label("󰟢")
                .build();

            workspace_box.append(&window_button);
        }

        for window in &workspace.windows {
            // creating box to be child of button
            // check if active or not
            let css_name = if window.order == 0 && workspace.order == 0 {
                "active-window-box".to_owned()
            } else {
                "window-box".to_owned()
            };

            let window_button = Button::builder()
                .css_name(&css_name)
                .css_classes(["bar-button"])
                .tooltip_text(&format!("{}", window.info))
                .build();

            // box has icon then label
            let icon_label_box = Box::builder().spacing(6).build();

            let icon = Image::builder()
                .file(&format!("assets/apps/{}.svg", window.class))
                .css_name("icon-image")
                .pixel_size(18)
                .build();

            icon_label_box.append(&icon);

            window_button.set_child(Some(&icon_label_box));
            let address = window.address.clone();
            // switch to clicked workspace
            window_button.connect_clicked(move |_| workspaces::switch_window(&address));

            let window_label = task_label(&window.name);

            icon_label_box.append(&window_label);

            workspace_box.append(&window_button);
        }
        container.append(&workspace_box);
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

fn task_scroller(container: &Box) -> gtk::ScrolledWindow {
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
            let entry = entry?;
            let path = entry.path();
            let path_name = path.to_str().to_owned().unwrap();
            return Ok(path_name.contains("BAT"));
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn assert_refined_widgets() {
        let icons = gtk::IconTheme::for_display(&Display::default().unwrap());
        for name in [
            "utilities-system-monitor-symbolic",
            "drive-harddisk-symbolic",
            "view-more-symbolic",
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

        for (width, values) in [
            (5, vec!["1%", "99%", "100%"]),
            (8, vec!["999 MHz", "5.2 GHz", "12.0 GHz"]),
            (17, vec!["1.0/8.0 GiB", "128.0/128.0 GiB"]),
            (8, vec!["01:11:11", "23:59:59"]),
        ] {
            let label = metric_label("cpu-load-label", width);
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
                if let Some(previous) = previous {
                    assert_eq!(natural, previous, "metric width changed for {value}");
                }
                previous = Some(natural);
            }
        }

        let cpu = metric_label("cpu-label", 8);
        let memory = metric_label("mem-label", 5);
        let detail = metric_label("mem-label", 17);
        cpu.set_visible(false);
        detail.set_visible(false);
        let toggle = ToggleButton::new();
        connect_status_toggle(&toggle, &cpu, &memory, &detail);
        toggle.set_active(true);
        assert!(cpu.is_visible() && detail.is_visible() && !memory.is_visible());
        toggle.set_active(false);
        assert!(!cpu.is_visible() && !detail.is_visible() && memory.is_visible());

        let group = Box::builder().css_classes(["status-group"]).build();
        let icon = status_icon("drive-harddisk-symbolic");
        group.append(&icon);
        group.append(&memory);
        let (_, normal_width, _, _) = group.measure(Orientation::Horizontal, -1);
        for (level, class, name) in [
            (StatusLevel::Warning, "warning", "dialog-warning-symbolic"),
            (StatusLevel::Critical, "critical", "dialog-error-symbolic"),
            (StatusLevel::Normal, "", "drive-harddisk-symbolic"),
        ] {
            update_status(&group, &icon, level, "drive-harddisk-symbolic");
            assert_eq!(icon.icon_name().as_deref(), Some(name));
            assert_eq!(group.has_css_class("warning"), class == "warning");
            assert_eq!(group.has_css_class("critical"), class == "critical");
            let (_, width, _, _) = group.measure(Orientation::Horizontal, -1);
            assert_eq!(width, normal_width, "alert state changed group width");
            let (_, height, _, _) = group.measure(Orientation::Vertical, -1);
            assert!(height <= 35, "status group is too tall: {height}px");
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

        for (name, expected) in [("top-bar", 36..=38), ("bottom-bar", 38..=40)] {
            let panel = Box::builder().css_name(name).build();
            let row = Box::builder().css_name("main-box").build();
            let button = Button::builder()
                .css_classes(["bar-button"])
                .label("Workspace")
                .build();
            row.append(&button);
            panel.append(&row);
            let (_, height, _, _) = panel.measure(Orientation::Vertical, -1);
            assert!(expected.contains(&height), "{name} height is {height}px");
        }
    }
}
