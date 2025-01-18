/*
   Copyright (C) 2025  Dylan Dy OR Dylan-Matthew Garza

   This program is free software: you can redistribute it and/or modify
   it under the terms of the GNU General Public License as published by
   the Free Software Foundation, either version 3 of the License, or
   (at your option) any later version.

   This program is distributed in the hope that it will be useful,
   but WITHOUT ANY WARRANTY; without even the implied warranty of
   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
   GNU General Public License for more details.

   You should have received a copy of the GNU General Public License
   along with this program.  If not, see <https://www.gnu.org/licenses/>.
   */
use std::{
    thread,
    sync::mpsc,
};

use gtk::{
    prelude::*,
    Application,
    ApplicationWindow,
    Box,
    Revealer,
    RevealerTransitionType,
    Button,
    Orientation,
    Align,
    gio,
    gdk::Display,
    pango::{
        FontDescription,
        Weight,
        SCALE,
        AttrFontDesc,
        AttrList
    },
    glib,
};

use gtk4_layer_shell::{
    LayerShell,
    Layer,
    Edge,
};

mod workspaces;
mod status;

const APP_ID: &str = "org.gtk_rs.epic_bar";
// TODO: figure out how to not repeat fonts in css classes/ set universal font
const CSS_DEFAULT: &str = "$icon_size: 20px; $font_size: 12px; \
                           window { font-family: 'Cascadia Code', sans-serif;} \
                           button {  font-family: 'Cascadia Code NF', sans-serif; border-radius: 0px; margin: 0px; padding: 0px 4px } \
                           .active { background-color:#4BA3FF; color: #fbf1c7; transition: 0.05s ease-in-out;}";

const BATTERY_ICON_TABLE: [&str;5] = [ "","","","",""];

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(top_bar);

    app.run()
}

fn top_bar(app: &Application) {

    // default css props
    let css_prov = gtk::CssProvider::new(); 
    css_prov.load_from_string(CSS_DEFAULT);

    init_style(&css_prov);

    let main_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::BaselineFill)
        .hexpand(true)
        .hexpand_set(true)
        .homogeneous(false)
        .css_name("main-box")
        .build();

    main_container.set_hexpand(true);

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
        .label("󰁚")
        .build();


    let battery_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .hexpand(false)
        .build();

    let battery_icon = Button::builder()
        .label("")
        .css_name("battery-icon")
        .name("battery-icon")
        .build();
    
    // status revealer should only contain label

    let battery_label = Button::builder()
        .label("N/A")
        .vexpand(false)
        .css_name("battery-label")
        .name("battery-label")
        .build();

    battery_container.append(&battery_icon);
    battery_container.append(&battery_label);

    status_revealed_container.append(&status_reveal_button);

    status_revealer.set_child(Some(&status_revealed_container));

    let sr_clone = status_revealer.clone();

    status_reveal_button.connect_clicked(move |btn| {
        if !sr_clone.is_child_revealed() {
            btn.set_label("󰬭");
        } else {
            btn.set_label("󰬧");
        }

        // update internal revealers
        sr_clone.set_reveal_child(!sr_clone.is_child_revealed());
    });

    status_container.append(&status_reveal_button);
    status_container.append(&status_revealer);

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

    // create window and set title
    let window = ApplicationWindow::builder()
        .application(app)
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

    let battery_icon = battery_icon.clone();
    let battery_label = battery_label.clone();

    // check if workspace activity in different thread to avoid blocking
    thread::spawn(move || {

        thread::sleep(std::time::Duration::from_millis(25));

        if workspaces::is_activity() {
            tx.send(()).unwrap();
        }

    });


    // on main threadm check if signal recieved that there is to update 
    // lets update everything else here too
    glib::source::idle_add_local(move || {
        thread::sleep(std::time::Duration::from_millis(25));
        if let Ok(_) = rx.try_recv() {
            populate_workspace_box(&workspace_clone);
        }
        let battery = status::get_battery();
        battery_label.set_label(&status::get_battery());

        glib::ControlFlow::Continue
    });

}

fn init_style(provider: &impl IsA<gtk::StyleProvider>) {
    let display = Display::default();
    gtk::style_context_add_provider_for_display(
        &display.unwrap(),
        provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
}

fn populate_workspace_box(workspace_container: &gtk::Box){

    let workspaces = workspaces::get_workspaces();
    let mut ws_opt = workspace_container.first_child();

    while let Some(ref workspace) = ws_opt {
        let tag:usize = workspace.widget_name().as_str().parse().unwrap();
        let workspace_info_opt = workspaces.get(&tag);
        if let Some(workspace_info) = workspace_info_opt {
            workspace.set_visible(true);
            if(workspace_info.order == 0) {
                workspace.add_css_class("active");
            }
            else {
                workspace.remove_css_class("active");
            }
        }
        else {
            workspace.set_visible(false);
        }
        ws_opt = workspace.next_sibling();
    }

}
