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

use glib::clone;

use gtk::{
    prelude::*,
    glib,
    Application,
    ApplicationWindow,
    Button,
    Label,
    Box,
    Orientation,
    CssProvider,
    StyleContext,
    gdk::{
        Display,
    
    },
    pango::{
        FontDescription,
        Weight,
        SCALE,
        AttrFontDesc,
        AttrList
    }
    
};

use gtk4_layer_shell::{
    LayerShell,
    Layer,
    Edge,
};

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


    let label = Label::builder()
        .label("0")
        .build();

    label.set_attributes(Some(&attr_list));


    let button = Button::builder()
        .child(&label)
        .build();

    button.style_context()
        .add_provider(&css_prov,gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

    let label2 = Label::builder()
        .label("0")
        .build();

    label2.set_attributes(Some(&attr_list));



    let button2 = Button::builder()
        .child(&label2)
        .build();

    let number = Rc::new(Cell::new(0));

    button.connect_clicked(clone!(
        #[weak]
        number,
        #[strong]
        label,
        #[strong]
        label2,
        move |_| {
            number.set(number.get() + 1);
            label.set_label(&number.get().to_string());
            label2.set_label(&number.get().to_string());
        }
    ));

    button2.connect_clicked(clone!(
        #[strong]
        label2,
        #[strong]
        label,
        move |_| {
            number.set(number.get() - 1);
            label2.set_label(&number.get().to_string());
            label.set_label(&number.get().to_string());
        }
    ));

    main_container.append(&button);
    main_container.append(&button2);


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
    gtk::style_context_add_provider_for_display(&display.unwrap(),provider,gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
}
