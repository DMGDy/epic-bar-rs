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

const APP_ID: &str = "org.gtk_rs.HelloWorld";
    
fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();

    app.connect_activate(build_ui);

    app.run()
}

fn build_ui(app: &Application) {

    let mut font = FontDescription::new();
    font.set_family("Monospace");
    font.set_size(8*SCALE);
    font.set_weight(Weight::Normal);

    let attr = AttrFontDesc::new(&font);

    let attr_list = AttrList::new();
    attr_list.insert(attr);

    let label = Label::builder()
        .label("0")
        .build();

    label.set_attributes(Some(&attr_list));


    let button = Button::builder()
        .child(&label)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(12)
        .margin_end(12)
        .build();

    let number = Rc::new(Cell::new(0));

    button.connect_clicked(clone!(
        #[strong]
        button,
        #[strong]
        label,
        move |_| {
            number.set(number.get() + 1);
            label.set_label(&number.get().to_string());
        }
    ));

    // create window and set title
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Hello World")
        .child(&button)
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
