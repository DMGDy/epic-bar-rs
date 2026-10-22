use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};

use gtk::{glib, prelude::*, Box, Button, DrawingArea, Label, MenuButton, Orientation, Popover};

use crate::network;

const SAMPLES: usize = 60;

fn menu(child: &impl IsA<gtk::Widget>, body: &impl IsA<gtk::Widget>, name: &str) -> MenuButton {
    let popover = Popover::builder()
        .css_classes(["bar-popover"])
        .position(gtk::PositionType::Bottom)
        .autohide(true)
        .has_arrow(true)
        .child(body)
        .build();
    let button = MenuButton::builder()
        .css_name(name)
        .css_classes(["status-popup"])
        .direction(gtk::ArrowType::None)
        .has_frame(false)
        .child(child)
        .popover(&popover)
        .build();
    button.connect_active_notify(|button| {
        if button.is_active() {
            button.add_css_class("popup-open");
        } else {
            button.remove_css_class("popup-open");
        }
    });
    button
}

fn body(title: &str) -> Box {
    let body = Box::new(Orientation::Vertical, 10);
    let heading = Label::builder()
        .label(title)
        .css_classes(["popup-heading"])
        .xalign(0.0)
        .build();
    body.append(&heading);
    body
}

pub fn calendar(child: &impl IsA<gtk::Widget>) -> MenuButton {
    let body = body("Calendar");
    let calendar = gtk::Calendar::builder().show_day_names(true).build();
    let today = Button::builder()
        .label("Today")
        .css_classes(["popover-action"])
        .build();
    let weak = calendar.downgrade();
    today.connect_clicked(move |_| {
        if let Some(calendar) = weak.upgrade() {
            match glib::DateTime::now_local() {
                Ok(now) => calendar.select_day(&now),
                Err(error) => eprintln!("epic-bar-rs: cannot select today's date: {error}"),
            }
        }
    });
    body.append(&calendar);
    body.append(&today);
    let button = menu(child, &body, "date-container");
    button.add_css_class("bar-button");
    button
}

#[derive(Default)]
struct History {
    samples: VecDeque<Option<f64>>,
}

impl History {
    fn push(&mut self, value: Option<f64>) {
        let value = value.filter(|value| {
            if value.is_finite() && (0.0..=100.0).contains(value) {
                true
            } else {
                eprintln!("epic-bar-rs: invalid usage-history sample: {value}");
                false
            }
        });
        if self.samples.len() == SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(value);
    }

    fn summary(&self) -> String {
        let values: Vec<_> = self.samples.iter().flatten().copied().collect();
        if values.is_empty() {
            return "Waiting for valid samples".into();
        }
        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let avg = values.iter().sum::<f64>() / values.len() as f64;
        format!("Min {min:.0}%   Avg {avg:.0}%   Max {max:.0}%")
    }
}

pub struct Usage {
    pub button: MenuButton,
    detail: Label,
    summary: Label,
    graph: DrawingArea,
    history: Rc<RefCell<History>>,
}

impl Usage {
    pub fn new(child: &impl IsA<gtk::Widget>, title: &str, class: &str) -> Self {
        let body = body(title);
        let detail = Label::builder().xalign(0.0).build();
        let summary = Label::builder()
            .label("Waiting for valid samples")
            .xalign(0.0)
            .css_classes(["popup-secondary"])
            .build();
        let graph = DrawingArea::builder()
            .content_width(260)
            .content_height(72)
            .css_classes(["usage-history", class])
            .build();
        let history = Rc::new(RefCell::new(History::default()));
        let draw_history = history.clone();
        graph.set_draw_func(move |graph, context, width, height| {
            if let Err(error) = draw_history_plot(
                context,
                width,
                height,
                &draw_history.borrow(),
                graph.color(),
            ) {
                eprintln!("epic-bar-rs: cannot draw usage history: {error}");
            }
        });
        body.append(&detail);
        body.append(&graph);
        body.append(&summary);
        body.append(
            &Label::builder()
                .label("Last 60 samples - one per second")
                .css_classes(["popup-secondary"])
                .xalign(0.0)
                .build(),
        );
        Self {
            button: menu(child, &body, "menubutton"),
            detail,
            summary,
            graph,
            history,
        }
    }

    pub fn update(&self, value: Option<f64>, detail: &str) {
        self.detail.set_text(detail);
        let mut history = self.history.borrow_mut();
        history.push(value);
        self.summary.set_text(&history.summary());
        self.graph.queue_draw();
    }
}

fn draw_history_plot(
    context: &gtk::cairo::Context,
    width: i32,
    height: i32,
    history: &History,
    color: gtk::gdk::RGBA,
) -> Result<(), gtk::cairo::Error> {
    let width = f64::from(width).max(1.0);
    let height = f64::from(height).max(1.0);
    context.set_source_rgba(
        color.red().into(),
        color.green().into(),
        color.blue().into(),
        0.15,
    );
    context.set_line_width(1.0);
    for y in [0.5, height / 2.0, height - 0.5] {
        context.move_to(0.0, y);
        context.line_to(width, y);
    }
    context.stroke()?;
    context.set_source_rgba(
        color.red().into(),
        color.green().into(),
        color.blue().into(),
        1.0,
    );
    context.set_line_width(2.0);
    let mut connected = false;
    for (index, value) in history.samples.iter().enumerate() {
        if let Some(value) = value {
            let x = (SAMPLES - history.samples.len() + index) as f64 / (SAMPLES - 1) as f64
                * (width - 4.0)
                + 2.0;
            let y = 2.0 + (height - 4.0) * (1.0 - value / 100.0);
            if connected {
                context.line_to(x, y);
            } else {
                context.move_to(x, y);
            }
            connected = true;
        } else {
            context.stroke()?;
            connected = false;
        }
    }
    context.stroke()
}

pub fn network(child: &impl IsA<gtk::Widget>, network: &network::Network) -> MenuButton {
    let body = body(&network.tooltip());
    let details = Label::builder()
        .label("Open to load connection details")
        .selectable(true)
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(48)
        .build();
    let copy = Button::builder()
        .label("Copy addresses")
        .css_classes(["popover-action"])
        .sensitive(false)
        .build();
    let copied = Label::builder()
        .xalign(0.0)
        .css_classes(["popup-secondary"])
        .build();
    body.append(&details);
    body.append(&copy);
    body.append(&copied);
    let button = menu(child, &body, "menubutton");
    let popover = button.popover().unwrap();
    let addresses = Rc::new(RefCell::new(String::new()));
    let copy_addresses = addresses.clone();
    let weak_copied = copied.downgrade();
    copy.connect_clicked(move |button| {
        button.clipboard().set_text(&copy_addresses.borrow());
        if let Some(copied) = weak_copied.upgrade() {
            copied.set_text("Addresses copied");
        }
    });
    let iface = network.iface.clone();
    let offline = network.kind == network::Kind::Offline;
    let weak_details = details.downgrade();
    let weak_copy = copy.downgrade();
    let weak_copied = copied.downgrade();
    let generation = Rc::new(Cell::new(0u64));
    popover.connect_show(move |_| {
        if offline {
            if let Some(details) = weak_details.upgrade() {
                details.set_text("No online physical network interfaces");
            }
            return;
        }
        let serial = generation.get().wrapping_add(1);
        generation.set(serial);
        if let Some(details) = weak_details.upgrade() {
            details.set_text("Loading connection details...");
        }
        if let Some(copy) = weak_copy.upgrade() {
            copy.set_sensitive(false);
        }
        if let Some(copied) = weak_copied.upgrade() {
            copied.set_text("");
        }
        let (weak_details, weak_copy, addresses, generation) = (
            weak_details.clone(),
            weak_copy.clone(),
            addresses.clone(),
            generation.clone(),
        );
        let iface = iface.clone();
        glib::spawn_future_local(async move {
            let result = gtk::gio::spawn_blocking(move || network::read_details(&iface)).await;
            if generation.get() != serial {
                return;
            }
            let Some(label) = weak_details.upgrade() else {
                return;
            };
            let result = match result {
                Ok(result) => result,
                Err(error) => Err(format!("network detail reader failed: {error:?}")),
            };
            match result {
                Ok(result) => {
                    label.set_text(&result.text());
                    *addresses.borrow_mut() = result.addresses.join("\n");
                    if let Some(copy) = weak_copy.upgrade() {
                        copy.set_sensitive(!addresses.borrow().is_empty());
                    }
                }
                Err(error) => {
                    eprintln!("epic-bar-rs: cannot read network details: {error}");
                    label.set_text(&format!("Cannot read connection details:\n{error}"));
                }
            }
        });
    });
    button
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_bounded_and_keeps_missing_samples_as_gaps() {
        let mut history = History::default();
        assert_eq!(history.summary(), "Waiting for valid samples");
        history.push(Some(f64::NAN));
        assert_eq!(history.samples[0], None);
        for value in 0..65 {
            history.push(Some(f64::from(value)));
        }
        assert_eq!(history.samples.len(), SAMPLES);
        assert_eq!(history.samples[0], Some(5.0));
        assert_eq!(history.summary(), "Min 5%   Avg 34%   Max 64%");
        history.push(None);
        assert_eq!(history.samples.back(), Some(&None));
    }
}

#[cfg(test)]
pub(crate) fn assert_widgets() {
    let group = Box::new(Orientation::Horizontal, 4);
    group.add_css_class("status-group");
    group.append(&crate::status_icon("drive-harddisk-symbolic"));
    group.append(&crate::metric_label("mem-label"));
    let before = group.measure(Orientation::Horizontal, -1).1;
    let usage = Usage::new(&group, "Memory", "memory-history");
    assert_eq!(usage.button.measure(Orientation::Horizontal, -1).1, before);
    usage.update(Some(42.0), "42% - 10/24 GiB");
    assert_eq!(usage.detail.text(), "42% - 10/24 GiB");
    assert_eq!(usage.summary.text(), "Min 42%   Avg 42%   Max 42%");
    assert!(usage.button.popover().unwrap().is_autohide());
    assert_eq!(
        usage.button.popover().unwrap().position(),
        gtk::PositionType::Bottom
    );
    assert!(usage.graph.height_request() <= 72);
    let surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 260, 72).unwrap();
    draw_history_plot(
        &gtk::cairo::Context::new(&surface).unwrap(),
        260,
        72,
        &usage.history.borrow(),
        usage.graph.color(),
    )
    .unwrap();
}
