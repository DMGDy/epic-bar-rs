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
    rc::Rc,
    cell::Cell,
    clone::Clone,
};

use gtk::{
    prelude::*,
    Application,
    ApplicationWindow,
    Button,
    Label,
    Box,
    Orientation,
    CssProvider,
    gdk::Display,
    pango::{
        FontDescription,
        Weight,
        SCALE,
        AttrFontDesc,
        AttrList
    },
    glib
    
};

use gtk4_layer_shell::{
    LayerShell,
    Layer,
    Edge,
};

mod workspaces;

const FONT_SIZE: i32 = 12;
const APP_ID: &str = "org.gtk_rs.epic_bar";
const BUTTON_DEFAULT: &str = "button { border-radius: 0px; }";
    
fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();

    app.connect_activate(top_bar);

    app.run()
}

fn top_bar(app: &Application) {

    // default css props
    let css_prov = gtk::CssProvider::new(); 
    css_prov.load_from_string(BUTTON_DEFAULT);

    init_style(&css_prov);

    // set font
    let mut font = FontDescription::new();
    font.set_family("Cascadia Code NF");
    font.set_size(FONT_SIZE*SCALE);
    font.set_weight(Weight::Normal);

    let attr = AttrFontDesc::new(&font);

    let attr_list = AttrList::new();
    attr_list.insert(attr);

    let main_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(1)
        .tooltip_text("test box")
        .css_name("main-box")
        .build();

    let workspace_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(0)
        .css_name("workspaces-container")
        .build();

    // init the container
    for n in 1..workspaces::WORKSPACE_COUNT+1 {
        let workspace_button = gtk::Button::builder()
            .label(format!("{}",n))
            .name(format!("{}",n))
            .visible(false)
            .build();
        workspace_container.append(&workspace_button);
    }

    populate_workspace_box(&workspace_container);
    let workspace_container_copy = workspace_container.clone();

    glib::MainContext::default().spawn_local(async move {
        loop {
            if workspaces::is_activity().await {
                let workspace_copy_copy = workspace_container_copy.clone();
                glib::idle_add_local_once(move || {
                    populate_workspace_box(&workspace_copy_copy);
                });
            }
            glib::timeout_future(std::time::Duration::from_millis(50)).await;
        }
    });

    main_container.append(&workspace_container);



    println!("here");


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



}

fn init_style(provider: &impl IsA<gtk::StyleProvider>) {
    let display = gtk::gdk::Display::default();
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
        if let Some(_) = workspace_info_opt {
                workspace.set_visible(true);
        }
        else {
            workspace.set_visible(false);
        }
        ws_opt = workspace.next_sibling();
    }

}
