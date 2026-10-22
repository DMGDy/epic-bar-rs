use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{
    glib, prelude::*, DrawingArea, EventControllerMotion, Fixed, Label, Orientation, Overlay,
    TickCallbackId,
};

const PAUSE_US: f64 = 1_000_000.0;
const PIXELS_PER_SECOND: f64 = 32.0;

#[derive(Clone, Copy)]
pub enum Mode {
    Continuous,
    Hover,
}

struct State {
    mode: Mode,
    label: Label,
    full: Label,
    canvas: Fixed,
    hovered: Cell<bool>,
    started: Cell<Option<i64>>,
    pending_layout: Cell<bool>,
    size: Cell<(i32, i32)>,
    tick: RefCell<Option<TickCallbackId>>,
    settings_handler: RefCell<Option<(gtk::Settings, glib::SignalHandlerId)>>,
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some((settings, handler)) = self.settings_handler.get_mut().take() {
            settings.disconnect(handler);
        }
    }
}

#[derive(Clone)]
pub struct Marquee {
    widget: Overlay,
    state: Rc<State>,
}

impl Marquee {
    pub fn new(label: Label, mode: Mode) -> Self {
        let full = Label::builder()
            .label(label.text())
            .css_name(label.css_name())
            .css_classes(label.css_classes())
            .single_line_mode(true)
            .xalign(0.0)
            .can_target(false)
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        let canvas = Fixed::builder().can_target(false).opacity(0.0).build();
        canvas.put(&full, 0.0, 0.0);
        let resize = DrawingArea::builder().can_target(false).build();
        let widget = Overlay::builder()
            .child(&label)
            .overflow(gtk::Overflow::Hidden)
            .build();
        // Only the original ellipsized label contributes to the bar's geometry.
        widget.add_overlay(&resize);
        widget.add_overlay(&canvas);
        widget.set_measure_overlay(&resize, false);
        widget.set_measure_overlay(&canvas, false);
        widget.set_clip_overlay(&canvas, true);
        let state = Rc::new(State {
            mode,
            label,
            full,
            canvas,
            hovered: Cell::new(false),
            started: Cell::new(None),
            pending_layout: Cell::new(true),
            size: Cell::new((0, 0)),
            tick: RefCell::new(None),
            settings_handler: RefCell::new(None),
        });
        let map_state = state.clone();
        widget.connect_map(move |widget| restart(widget, &map_state));
        let unmap_state = state.clone();
        widget.connect_unmap(move |_| {
            unmap_state.hovered.set(false);
            stop(&unmap_state);
        });
        let weak_widget = widget.downgrade();
        let resize_state = state.clone();
        resize.connect_resize(move |_, width, height| {
            if resize_state.size.replace((width, height)) != (width, height) {
                if let Some(widget) = weak_widget.upgrade() {
                    restart(&widget, &resize_state);
                }
            }
        });
        let weak_widget = widget.downgrade();
        let weak_state = Rc::downgrade(&state);
        let settings = widget.settings();
        let handler = settings.connect_notify_local(Some("gtk-enable-animations"), move |_, _| {
            if let (Some(widget), Some(state)) = (weak_widget.upgrade(), weak_state.upgrade()) {
                restart(&widget, &state);
            }
        });
        *state.settings_handler.borrow_mut() = Some((settings, handler));
        Self { widget, state }
    }

    pub fn widget(&self) -> &Overlay {
        &self.widget
    }

    pub fn set_text(&self, text: &str) {
        if self.state.label.text() == text {
            return;
        }
        self.state.label.set_text(text);
        self.state.full.set_text(text);
        restart(&self.widget, &self.state);
    }

    pub fn track_hover(&self, target: &impl IsA<gtk::Widget>) {
        let controller = EventControllerMotion::new();
        let weak_widget = self.widget.downgrade();
        let weak_state = Rc::downgrade(&self.state);
        controller.connect_enter(move |_, _, _| {
            if let (Some(widget), Some(state)) = (weak_widget.upgrade(), weak_state.upgrade()) {
                state.hovered.set(true);
                restart(&widget, &state);
            }
        });
        let weak_state = Rc::downgrade(&self.state);
        controller.connect_leave(move |_| {
            if let Some(state) = weak_state.upgrade() {
                state.hovered.set(false);
                stop(&state);
            }
        });
        target.add_controller(controller);
    }

    #[cfg(test)]
    pub fn label(&self) -> &Label {
        &self.state.label
    }
}

fn active(widget: &Overlay, state: &State) -> bool {
    widget.settings().property::<bool>("gtk-enable-animations")
        && (matches!(state.mode, Mode::Continuous) || state.hovered.get())
}

fn distance(widget: &Overlay, state: &State) -> f64 {
    f64::from((state.full.measure(Orientation::Horizontal, -1).1 - widget.width()).max(0))
}

fn stop(state: &State) {
    let tick = state.tick.borrow_mut().take();
    if let Some(tick) = tick {
        tick.remove();
    }
    reset(state);
}

fn reset(state: &State) {
    state.started.set(None);
    state.label.set_opacity(1.0);
    state.canvas.set_opacity(0.0);
}

fn restart(widget: &Overlay, state: &Rc<State>) {
    stop(state);
    state.pending_layout.set(true);
    if !widget.is_mapped() || !active(widget, state) {
        return;
    }
    let tick_state = state.clone();
    let tick = widget.add_tick_callback(move |widget, clock| {
        if !widget.is_mapped() || !active(widget, &tick_state) {
            tick_state.tick.borrow_mut().take();
            reset(&tick_state);
            return glib::ControlFlow::Break;
        }
        if tick_state.pending_layout.replace(false) {
            return glib::ControlFlow::Continue;
        }
        if frame(widget, &tick_state, clock.frame_time()) {
            glib::ControlFlow::Continue
        } else {
            tick_state.tick.borrow_mut().take();
            reset(&tick_state);
            glib::ControlFlow::Break
        }
    });
    *state.tick.borrow_mut() = Some(tick);
}

fn frame(widget: &Overlay, state: &State, now: i64) -> bool {
    let distance = distance(widget, state);
    if !active(widget, state) || distance <= 0.0 {
        return false;
    }
    let started = state.started.get().unwrap_or(now);
    state.started.set(Some(started));
    let offset = offset_at(now - started, distance);
    let height = state.full.measure(Orientation::Vertical, -1).1;
    state.canvas.move_(
        &state.full,
        -offset,
        f64::from(widget.height() - height) / 2.0,
    );
    state.label.set_opacity(0.0);
    state.canvas.set_opacity(1.0);
    true
}

fn offset_at(elapsed: i64, distance: f64) -> f64 {
    if distance <= 0.0 {
        return 0.0;
    }
    let travel = distance / PIXELS_PER_SECOND * 1_000_000.0;
    let phase = (elapsed.max(0) as f64) % (2.0 * (PAUSE_US + travel));
    if phase <= PAUSE_US {
        0.0
    } else if phase <= PAUSE_US + travel {
        (phase - PAUSE_US) / travel * distance
    } else if phase <= 2.0 * PAUSE_US + travel {
        distance
    } else {
        (1.0 - (phase - 2.0 * PAUSE_US - travel) / travel) * distance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_pauses_at_both_ends_and_repeats_without_a_jump() {
        for (time, expected) in [
            (0, 0.0),
            (1_000_000, 0.0),
            (2_000_000, 32.0),
            (3_000_000, 64.0),
            (4_000_000, 64.0),
            (5_000_000, 32.0),
            (6_000_000, 0.0),
            (7_000_000, 0.0),
            (8_000_000, 32.0),
        ] {
            assert_eq!(offset_at(time, 64.0), expected);
        }
        assert_eq!(offset_at(-1, 64.0), 0.0);
        assert_eq!(offset_at(1_000_000, 0.0), 0.0);
    }
}

#[cfg(test)]
pub(crate) fn assert_one_pixel_overflow(title: &Marquee) {
    let settings = title.widget.settings();
    let animations = settings.property::<bool>("gtk-enable-animations");
    settings.set_property("gtk-enable-animations", true);
    let width = title.state.full.measure(Orientation::Horizontal, -1).1 - 1;
    let height = title.widget.measure(Orientation::Vertical, width).1;
    title.widget.allocate(width, height, -1, None);
    assert_eq!(distance(&title.widget, &title.state), 1.0);
    assert!(title.state.label.layout().is_ellipsized());
    assert!(
        frame(&title.widget, &title.state, 0),
        "one pixel of overflow still produces an ellipsis and must start scrolling"
    );
    assert!(frame(&title.widget, &title.state, 1_500_000));
    title.widget.allocate(width, height, -1, None);
    assert_eq!(title.state.label.opacity(), 0.0);
    assert_eq!(title.state.canvas.opacity(), 1.0);
    let end = title.state.full.compute_bounds(&title.widget).unwrap();
    assert!(
        (end.x() + end.width() - width as f32).abs() < 0.01,
        "the final glyph did not reach the viewport: {end:?}"
    );
    stop(&title.state);
    title.widget.allocate(width + 1, height, -1, None);
    assert_eq!(distance(&title.widget, &title.state), 0.0);
    assert!(!title.state.label.layout().is_ellipsized());
    assert!(!frame(&title.widget, &title.state, 2_000_000));
    settings.set_property("gtk-enable-animations", animations);
}

#[cfg(test)]
pub(crate) fn assert_widgets() {
    let settings = gtk::Settings::default().unwrap();
    let animations = settings.property::<bool>("gtk-enable-animations");
    settings.set_property("gtk-enable-animations", true);
    for mode in [Mode::Continuous, Mode::Hover] {
        let title = "Long title <b>plain text</b> and 日本語 ".repeat(12);
        let label = crate::task_label(&title);
        let expected_width = label.measure(Orientation::Horizontal, -1).1;
        let expected_height = label.measure(Orientation::Vertical, expected_width).1;
        let marquee = Marquee::new(label, mode);
        assert_eq!(
            marquee.widget.measure(Orientation::Horizontal, -1).1,
            expected_width
        );
        marquee
            .widget
            .allocate(expected_width, expected_height, -1, None);
        assert!(distance(&marquee.widget, &marquee.state) > 100.0);
        assert!(!marquee.state.full.layout().is_ellipsized());
        assert!(!marquee.state.full.uses_markup());
        assert_eq!(marquee.state.full.text(), title);
        assert!(
            marquee.state.tick.borrow().is_none(),
            "unmapped title started ticking"
        );
        let hover = if matches!(mode, Mode::Hover) {
            assert!(!frame(&marquee.widget, &marquee.state, 0));
            let target = gtk::Box::new(Orientation::Horizontal, 0);
            marquee.track_hover(&target);
            let controller = target
                .observe_controllers()
                .item(0)
                .unwrap()
                .downcast::<EventControllerMotion>()
                .unwrap();
            controller.emit_by_name::<()>("enter", &[&0.0f64, &0.0f64]);
            assert!(marquee.state.hovered.get());
            Some((target, controller))
        } else {
            None
        };
        assert!(frame(&marquee.widget, &marquee.state, 0));
        assert!(frame(&marquee.widget, &marquee.state, 2_000_000));
        marquee
            .widget
            .allocate(expected_width, expected_height, -1, None);
        assert_eq!(
            marquee.state.canvas.child_position(&marquee.state.full).0,
            -32.0
        );
        assert_eq!(
            marquee
                .state
                .full
                .compute_bounds(&marquee.widget)
                .unwrap()
                .x(),
            -32.0
        );
        let travel = distance(&marquee.widget, &marquee.state) / PIXELS_PER_SECOND;
        assert!(frame(
            &marquee.widget,
            &marquee.state,
            (PAUSE_US * 1.5 + travel * 1_000_000.0) as i64
        ));
        marquee
            .widget
            .allocate(expected_width, expected_height, -1, None);
        let end = marquee.state.full.compute_bounds(&marquee.widget).unwrap();
        assert!(
            (end.x() + end.width() - expected_width as f32).abs() < 0.01,
            "the end of the full title was not revealed: {end:?}"
        );
        assert_eq!(marquee.state.label.opacity(), 0.0);
        assert_eq!(marquee.state.canvas.opacity(), 1.0);
        assert_eq!(marquee.widget.width(), expected_width);
        assert_eq!(
            marquee.widget.measure(Orientation::Horizontal, -1).1,
            expected_width
        );
        marquee.set_text(&title);
        assert_eq!(
            marquee.state.started.get(),
            Some(0),
            "unchanged title reset scrolling"
        );
        settings.set_property("gtk-enable-animations", false);
        assert_eq!(marquee.state.canvas.opacity(), 0.0);
        assert_eq!(marquee.state.label.opacity(), 1.0);
        assert!(!frame(&marquee.widget, &marquee.state, 3_000_000));
        settings.set_property("gtk-enable-animations", true);
        if let Some((_, controller)) = &hover {
            controller.emit_by_name::<()>("leave", &[]);
            assert!(!marquee.state.hovered.get());
            assert_eq!(marquee.state.started.get(), None);
            assert_eq!(marquee.state.label.opacity(), 1.0);
            assert!(!frame(&marquee.widget, &marquee.state, 4_000_000));
            controller.emit_by_name::<()>("enter", &[&0.0f64, &0.0f64]);
        }
        marquee.set_text("X");
        assert_eq!(marquee.state.started.get(), None);
        let width = marquee.widget.measure(Orientation::Horizontal, -1).1;
        assert!(width < expected_width, "shorter title did not shrink");
        marquee.widget.allocate(width, expected_height, -1, None);
        assert!(!frame(&marquee.widget, &marquee.state, 0));
        let state = Rc::downgrade(&marquee.state);
        drop(marquee);
        assert!(state.upgrade().is_none(), "closed title retained its state");
    }
    settings.set_property("gtk-enable-animations", animations);
}
