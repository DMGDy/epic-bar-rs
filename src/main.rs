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
use std::{
    thread,
    time::Duration,
    sync::mpsc,
    rc::Rc,
    cell::Cell,
};

use gtk::{
    prelude::*,
    Application,
    ApplicationWindow,
    Box,
    Label,
    Button,
    Orientation,
    Align,
    gio,
    CssProvider,
    StyleProvider,
    Image,
    gdk::Display,
    glib::{
        clone,
        ControlFlow,
    },
    glib
};

use gtk4_layer_shell::{
    LayerShell,
    Layer,
    Edge,
};

mod workspaces;
mod status;
mod css;

const APP_ID: &str = "org.gtk_rs.epic_bar";
// This cannot keep going
fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(top_bar);
    app.connect_activate(bottom_bar);

    app.run()
}

fn top_bar(app: &Application) {

    // default css props
    let css_prov = CssProvider::new(); 
    css_prov.load_from_string(css::CSS);

    init_style(&css_prov);

    let main_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::BaselineFill)
        .hexpand(true)
        .hexpand_set(true)
        .homogeneous(false)
        .css_name("main-box")
        .build();

    let workspace_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(0)
        .css_name("workspaces-container")
        .build();

    let spacer = Box::builder()
        .orientation(Orientation::Horizontal)
        .hexpand(true)
        .build();

    let status_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .hexpand(false)
        .build();
    
    // button to reveal all statuses
    let status_reveal_button = Button::builder()
        .css_name("status-reveal-button")
        .label("󰁚")
        .build();

    let battery_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .hexpand(false)
        .build();

    let battery_image = Image::builder()
        .file("status/battery-missing.svg")
        .css_name("icon-image")
        .pixel_size(20)
        .build();

    // reveal text if clicked, or hovered
    let battery_icon = Button::builder()
        .css_name("battery-icon")
        .name("battery-icon")
        .visible(true)
        .build();

    let battery_label = Button::builder()
        .label("N/A")
        .hexpand(false)
        .visible(true)
        .css_name("battery-label")
        .name("battery-label")
        .build();

    let mem_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .hexpand(false)
        .build();

    let mem_label = Button::builder()
        .label("N/A")
        .hexpand(false)
        .visible(true)
        .css_name("mem-label")
        .build();

    let mem_icon = Button::builder()
        .css_name("mem-icon")
        .visible(true)
        .build();
        

    let date_container = Button::builder()
        .css_name("date-container")
        .build();

    let date_label= Label::builder()
        .lines(2)
        .css_name("date-label")
        .name("date-label")
        .label("date\ntime")
        .build();

    battery_icon.set_child(Some(&battery_image));
    battery_container.append(&battery_icon);
    battery_container.append(&battery_label);

    date_container.set_child(Some(&date_label));

    status_container.append(&status_reveal_button);
    status_container.append(&battery_container);

    let toggle = Rc::new(Cell::new(false));

    // close/open all status modules
    status_reveal_button.connect_clicked(clone!(
        #[weak]
        status_reveal_button,
        #[weak]
        battery_label,
        move |_| {
            if !toggle.get() {
                status_reveal_button.set_label("󰁋");
                toggle.set(true);
            } else {
                status_reveal_button.set_label("󰁚");
                toggle.set(false);
            }
            battery_label.set_visible(toggle.get());
        }
    ));

    // individual clicking of icon to expand
    battery_icon.connect_clicked(clone!(
        #[weak]
        battery_label,
        move |_| {
            battery_label.set_visible(!battery_label.get_visible());
        }
    ));

    // init the container
    for n in 1..workspaces::WORKSPACE_COUNT+1 {
        let workspace_button = Button::builder()
            .label(format!("{}",n))
            .name(format!("{}",n))
            .visible(true)
            .focus_on_click(true)
            .build();

        let nclone = n.clone();

        workspace_button.connect_clicked( move |_| {
            gio::spawn_blocking(move || {
                workspaces::switch_workspace(nclone);
            });
        });
        workspace_container.append(&workspace_button);
    };

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
    LayerShell::set_layer(&window,Layer::Top);

    LayerShell::auto_exclusive_zone_enable(&window);

    LayerShell::set_anchor(&window, Edge::Top, true);
    LayerShell::set_anchor(&window, Edge::Left, true);
    LayerShell::set_anchor(&window, Edge::Right, true);

    window.set_decorated(true);
    window.present();

    let (tx,rx) = mpsc::channel();

    let workspace_clone = workspace_container.clone();
    // check if workspace activity in different thread to avoid blocking
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_millis(50));
            if workspaces::is_activity() {
                tx.send(()).unwrap();
            }
        }
    });
    
    let battery_image = battery_image.clone();
    let battery_label = battery_label.clone();
    let date_container = date_container.clone();
    // on main thread check if signal recieved that there is to update 

    glib::source::timeout_add_local(Duration::from_millis(50),move || {

        let dt = status::get_datetime();

        date_container.set_label(&format!("{dt}"));

        if let Ok(_) = rx.try_recv() {
            populate_workspace_box(&workspace_clone);
        }

        ControlFlow::Continue
    });

    // update other stuff less frequently
    glib::source::timeout_add_seconds_local(1,move || {
        let battery = status::get_battery_info();
        let mut bl = battery.capacity.to_string();
        bl.push_str("%");
        battery_label.set_label(&bl);
        let svg_path = std::path::Path::new(&battery.icon);
        battery_image.set_from_file(Some(&svg_path));
        let tooltip_str = battery.tooltip_text;
        battery_image.set_tooltip_text(Some(&tooltip_str));
        
        ControlFlow::Continue
    });
}

fn init_style(provider: &impl IsA<StyleProvider>) {
    let display = Display::default();
    gtk::style_context_add_provider_for_display(
        &display.unwrap(),
        provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
}

fn populate_workspace_box(workspace_container: &Box){
    let workspaces = workspaces::get_workspaces();
    let mut ws_opt = workspace_container.first_child();

    while let Some(ref workspace) = ws_opt {
        let tag :usize = workspace.widget_name().as_str().parse().unwrap();
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
        }
        else {
            workspace.set_visible(false);
        }    
        ws_opt = workspace.next_sibling();
    }
}

fn bottom_bar(app: &Application) {
    
    let css_prov = CssProvider::new(); 
    css_prov.load_from_string(css::CSS);

    init_style(&css_prov);


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
        .css_name("workspace-windows-container")
        .build();

    // so minimum vertical space is occupied when displayed
    let reserve = Button::builder()
        .label(" ")
        .build();

    workspace_windows_container.append(&reserve);

    // needed for no shrinking in screenshot mode
    let fill = Box::builder()
        .hexpand(true)
        .halign(Align::BaselineFill)
        .build();

    let fill2 = Button::builder()
        .hexpand(true)
        .build();

    main_container.append(&workspace_windows_container);
    main_container.append(&fill);
    main_container.append(&fill2);
    let window = ApplicationWindow::builder()
        .css_name("bottom-bar")
        .resizable(true)
        .application(app)
        .child(&main_container)
        .build();
    LayerShell::init_layer_shell(&window);
    LayerShell::set_layer(&window,Layer::Top);

    LayerShell::auto_exclusive_zone_enable(&window);

    LayerShell::set_anchor(&window, Edge::Bottom, true);
    LayerShell::set_anchor(&window, Edge::Right, true);
    LayerShell::set_anchor(&window, Edge::Left, true);

    window.set_decorated(true);
    window.present();
    let (tx,rx) = mpsc::channel();

    // check if workspace activity in different thread to avoid blocking
    thread::spawn(move || {
        loop {
            thread::sleep(std::time::Duration::from_millis(25));
            if workspaces::is_activity() {
                tx.send(()).unwrap();
            }
        }
    });



    glib::source::idle_add_local(move || {
        thread::sleep(std::time::Duration::from_millis(25));

         if let Ok(_) = rx.try_recv() {
            populate_windows_container(&workspace_windows_container);
        }

        ControlFlow::Continue
    });

}

fn populate_windows_container(container: &Box) {
    let workspaces = workspaces::get_workspaces();

    let sorted: &mut Vec<_> = &mut workspaces
        .into_iter()
        .collect();

    // sort by recently used
    sorted.sort_by(|w1,w2| w1.1.order.cmp(&w2.1.order));

    let mut tag_opt = container.first_child();

    while let Some(tag) = tag_opt {
        // empty container with everything
        container.remove(&tag);   
        tag_opt = container.first_child()
    }


    // start filling with occupied workspaces
    for (tag,workspace) in sorted {
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
            .name(format!("{}",tag))
            .css_name(workspace_windows_css)
            .vexpand(true)
            .build();

        let tag_label = Label::builder()
            .label(format!("{}",tag))
            .css_name("tag-label")
            .build();

        workspace_box.append(&tag_label);

        if workspace.windows.is_empty() {

            let window_button = Button::builder()
                .css_name("window-box-empty")
                .label("󰟢")
                .build();

            workspace_box.append(&window_button);
        }


        for window in &workspace.windows {

            // creating box to be child of button
            // check if active or not
            let css_name = if window.order == 0 && workspace.order == 0 {
                "active-window-box".to_owned()
            } else  {
                    "window-box".to_owned()
            };

            let window_button = Button::builder()
                .css_name(&css_name)
                .tooltip_text(&format!("{}",window.info))
                .build();

            // box has icon then label
            let icon_label_box = Box::builder()
                .build();
            
            let icon = Image::builder()
                .file(&format!("icons/{}.svg",window.class))
                .css_name("icon-image")
                .pixel_size(20)
                .build();

            icon_label_box.append(&icon);

            window_button.set_child(Some(&icon_label_box));
            let address = window.address.clone();
            // switch to clicked workspace
            window_button.connect_clicked(move |_| {
                workspaces::switch_window(&address)
            });

            let window_label = Label::builder()
                .label(format!("{}",window.name))
                .css_name("window-label")
                .build();

            icon_label_box.append(&window_label);

            workspace_box.append(&window_button);
        }
        container.append(&workspace_box);
    }
}
