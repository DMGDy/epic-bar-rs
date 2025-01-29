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
    gdk::{
        Display,
        gdk_pixbuf::Pixbuf,
        MemoryTextureBuilder
    },
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

const APP_ID: &str = "org.gtk_rs.epic_bar";
// TODO: figure out how to not repeat fonts in css classes/ set universal font
const CSS_DEFAULT: &str = "\
                           window { font-family: 'Cascadia Code', sans-serif; } \
                           button { font-family: 'Cascadia Code NF', sans-serif; border-radius: 0px; margin: 0px; padding: 0px 5px; } \
                           box { font-family: 'Cascadia Code NF', sans-serif; border-radius: 0px; margin: 0px; padding: 0px 5px; } \
                           status-reveal-button { font-size:22px; border-right: 1px ridge white; padding: 0px 4px 0px 0px;} \
                           battery-icon { padding: 0px 2px; font-size: 20px; } \
                           battery-label { padding: 0px 0px; } \
                           .active { background-color:#4BA3FF; color: #fbf1c7; transition: color 1s; } \
                           date-container { border-left: 1px solid white; font-size: 10px; padding: 0px 4px; } \
                           ";

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(top_bar);
    app.connect_activate(bottom_bar);

    app.run()
}

fn top_bar(app: &Application) {

    // default css props
    let css_prov = CssProvider::new(); 
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

    // reveal text if clicked, or hovered
    let battery_icon = Button::builder()
        .label("")
        .css_name("battery-icon")
        .name("battery-icon")
        .visible(true)
        .build();
    
    let battery_label = Button::builder()
        .label("N/A")
        .vexpand(false)
        .visible(false)
        .css_name("battery-label")
        .name("battery-label")
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

    battery_container.append(&battery_icon);
    battery_container.append(&battery_label);

    date_container.set_child(Some(&date_label));

    status_container.append(&status_reveal_button);
    status_container.append(&battery_container);


    let toggle = Rc::new(Cell::new(false));

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
            thread::sleep(std::time::Duration::from_millis(25));
            if workspaces::is_activity() {
                tx.send(()).unwrap();
            }
        }
    });

    let battery_icon = battery_icon.clone();
    let battery_label = battery_label.clone();
    let date_container = date_container.clone();

    // on main threadm check if signal recieved that there is to update 
    // lets update everything else here too

    glib::source::idle_add_local(move || {


        thread::sleep(std::time::Duration::from_millis(25));
        let dt = status::get_datetime();

        date_container.set_label(&format!("{dt}"));

        if let Ok(_) = rx.try_recv() {
            populate_workspace_box(&workspace_clone);
        }

        let battery = status::get_battery_info();
        let mut bl = battery.capacity.to_string();
        bl.push_str("%");
        battery_label.set_label(&bl);

        battery_icon.set_label(&battery.icon);

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
        if let Some(workspace_info) = workspace_info_opt {
            workspace.set_visible(true);
            if workspace_info.active {
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

fn bottom_bar(app: &Application) {
    
    let css_prov = CssProvider::new(); 
    css_prov.load_from_string(CSS_DEFAULT);

    init_style(&css_prov);

    let main_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::BaselineFill)
        .hexpand(true)
        .vexpand(false)
        .hexpand_set(true)
        .homogeneous(false)
        .css_name("main-box")
        .build();

    let workspace_windows_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::BaselineFill)
        .hexpand(true)
        .vexpand(false)
        .homogeneous(false)
        .css_name("workspace-windows-container")
        .build();

    // so minimum vertical space is occupied when displayed
    let reserve = Button::builder()
        .label(" ")
        .build();

    workspace_windows_container.append(&reserve);



    main_container.append(&workspace_windows_container);
    let window = ApplicationWindow::builder()
        .application(app)
        .child(&main_container)
        .build();

    LayerShell::init_layer_shell(&window);
    LayerShell::set_layer(&window,Layer::Top);

    LayerShell::auto_exclusive_zone_enable(&window);

    LayerShell::set_anchor(&window, Edge::Bottom, true);
    LayerShell::set_anchor(&window, Edge::Left, true);
    LayerShell::set_anchor(&window, Edge::Right, true);

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
        let workspace_box = Box::builder()
            .name(format!("{}",tag))
            .vexpand(false)
            .build();

        let label = Label::builder()
            .label(format!("{} ",tag))
            .build();

        workspace_box.append(&label);

        if workspace.windows.is_empty() {
            let window_button = Button::builder()
                .css_name("window-box-empty")
                .label("󰟢")
                .build();

            workspace_box.append(&window_button);
        }

        for window in &workspace.windows {
            // have icon theme instead of css
            let window_button = Button::builder()
                .css_name(&format!("window-box.{}",window.class))
                .build();
            // create new css provider for background of button
            // setting icon wouldnt allow any child widgets
            
            let icon_label_box = Box::builder()
                .build();
            
            let path_str = format!("icons/{}.svg",window.class);

            // create pixbuf to bytes -> memorytexturebuilder->set height,width,stride then build
            // into Texture (implements Paintable trait) -> into Image (GtkWidget)
            let pixbuf = Pixbuf::from_file_at_size(&path_str,512,512).expect("incorrect file");
            let bytes = pixbuf.read_pixel_bytes();
            println!("{:?}",bytes.len());
            let texture_builder = MemoryTextureBuilder::new();

            texture_builder.set_bytes(Some(&bytes));
            texture_builder.set_height(512);
            texture_builder.set_width(512);
            texture_builder.set_stride(512*4);

            let texture = texture_builder.build();
            let icon = Image::from_paintable(Some(&texture));

            icon_label_box.append(&icon);

            window_button.set_child(Some(&icon_label_box));
            let address = window.address.clone();
            window_button.connect_clicked(move |_| {
                workspaces::switch_window(&address)
            });

            let window_label = Label::builder()
                .label(format!("{} ",window.name))
                .build();

            icon_label_box.append(&window_label);

            workspace_box.append(&window_button);
        }
        container.append(&workspace_box);
    }
    // set the added css styles
}
