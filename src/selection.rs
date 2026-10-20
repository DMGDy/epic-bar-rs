use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{cairo, glib::ControlFlow, prelude::*, Box, DrawingArea, Overlay};

const DURATION_US: i64 = 200_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn interpolate(self, target: Self, progress: f64) -> Self {
        let interpolate = |from, to| from + (to - from) * progress;
        Self {
            x: interpolate(self.x, target.x),
            y: interpolate(self.y, target.y),
            width: interpolate(self.width, target.width),
            height: interpolate(self.height, target.height),
        }
    }
}

#[derive(Default)]
pub(crate) struct Motion {
    pub current: Option<Rect>,
    from: Option<Rect>,
    target: Option<Rect>,
    started: i64,
}

impl Motion {
    pub fn advance(&mut self, target: Option<Rect>, now: i64, animate: bool) -> bool {
        if target != self.target {
            self.from = self.current;
            self.target = target;
            self.started = now;
        }
        let Some(target) = target else {
            self.current = None;
            return false;
        };
        if let Some(from) = self.from.filter(|_| animate) {
            let progress = ((now - self.started) as f64 / DURATION_US as f64).clamp(0.0, 1.0);
            self.current = Some(from.interpolate(target, 1.0 - (1.0 - progress).powi(3)));
            if progress >= 1.0 {
                self.from = None;
            }
            progress < 1.0
        } else {
            self.current = Some(target);
            self.from = None;
            false
        }
    }
}

#[derive(Default)]
struct State {
    motion: RefCell<Motion>,
    ticking: Cell<bool>,
    waiting_for_layout: Cell<bool>,
    layout: RefCell<Vec<(bool, bool)>>,
}

pub struct Selection {
    overlay: Overlay,
    canvas: DrawingArea,
    container: Box,
    state: Rc<State>,
}

impl Selection {
    pub fn new(container: &Box) -> Self {
        let state = Rc::new(State::default());
        *state.layout.borrow_mut() = layout_key(container);
        let canvas = DrawingArea::builder().can_target(false).build();
        let overlay = Overlay::builder()
            .css_classes(["animated-workspaces"])
            .child(&canvas)
            .build();
        overlay.add_overlay(container);
        overlay.set_measure_overlay(container, true);
        overlay.set_clip_overlay(container, true);

        let draw_state = state.clone();
        canvas.set_draw_func(move |canvas, context, _, _| {
            let Some(rect) = draw_state.motion.borrow().current else {
                return;
            };
            // Other themes retain their ordinary CSS button highlight.
            if let Some((fill, accent)) = colors(canvas) {
                if let Err(error) = draw(context, rect, &fill, &accent) {
                    eprintln!("epic-bar-rs: cannot draw workspace selection: {error}");
                }
            }
        });
        let map_state = state.clone();
        let map_container = container.clone();
        canvas.connect_map(move |canvas| start(canvas, &map_container, &map_state));
        let resize_state = state.clone();
        let resize_container = container.clone();
        canvas.connect_resize(move |canvas, _, _| start(canvas, &resize_container, &resize_state));

        Self {
            overlay,
            canvas,
            container: container.clone(),
            state,
        }
    }

    pub fn widget(&self) -> &Overlay {
        &self.overlay
    }

    pub fn refresh(&self) {
        let layout = layout_key(&self.container);
        if *self.state.layout.borrow() != layout {
            *self.state.layout.borrow_mut() = layout;
            start(&self.canvas, &self.container, &self.state);
        }
    }
}

fn layout_key(container: &Box) -> Vec<(bool, bool)> {
    let mut key = Vec::new();
    let mut child = container.first_child();
    while let Some(button) = child {
        key.push((
            button.is_visible(),
            button.has_css_class("active") || button.has_css_class("empty-active"),
        ));
        child = button.next_sibling();
    }
    key
}

fn active_rect(container: &Box, canvas: &DrawingArea) -> Option<Rect> {
    let mut child = container.first_child();
    while let Some(button) = child {
        if button.is_visible()
            && (button.has_css_class("active") || button.has_css_class("empty-active"))
        {
            let bounds = button.compute_bounds(canvas)?;
            if bounds.width() > 0.0 && bounds.height() > 0.0 {
                return Some(Rect {
                    x: f64::from(bounds.x()),
                    y: f64::from(bounds.y()),
                    width: f64::from(bounds.width()),
                    height: f64::from(bounds.height()),
                });
            }
        }
        child = button.next_sibling();
    }
    None
}

fn start(canvas: &DrawingArea, container: &Box, state: &Rc<State>) {
    if !canvas.is_mapped() {
        return;
    }
    state.waiting_for_layout.set(true);
    if state.ticking.replace(true) {
        return;
    }
    let state = state.clone();
    let container = container.clone();
    // The next layout must finish before reading positions of newly visible buttons.
    canvas.add_tick_callback(move |canvas, clock| {
        if state.waiting_for_layout.replace(false) {
            return ControlFlow::Continue;
        }
        let animate = canvas.settings().property::<bool>("gtk-enable-animations");
        let moving = state.motion.borrow_mut().advance(
            active_rect(&container, canvas),
            clock.frame_time(),
            animate,
        );
        canvas.queue_draw();
        if moving {
            ControlFlow::Continue
        } else {
            state.ticking.set(false);
            ControlFlow::Break
        }
    });
}

// GTK 4.16 still needs StyleContext to resolve named runtime CSS colors.
#[allow(deprecated)]
fn colors(canvas: &DrawingArea) -> Option<(gtk::gdk::RGBA, gtk::gdk::RGBA)> {
    let style = canvas.style_context();
    Some((
        style.lookup_color("autumn_selection")?,
        style.lookup_color("autumn_accent")?,
    ))
}

fn draw(
    context: &cairo::Context,
    rect: Rect,
    fill: &gtk::gdk::RGBA,
    accent: &gtk::gdk::RGBA,
) -> Result<(), cairo::Error> {
    let Rect {
        x,
        y,
        width,
        height,
    } = rect;
    context.save()?;
    context.rectangle(x, y, width, height);
    context.clip();
    context.set_source_rgba(
        fill.red().into(),
        fill.green().into(),
        fill.blue().into(),
        fill.alpha().into(),
    );
    context.paint()?;
    context.rectangle(x, y + height - 2.0, width, 2.0);
    context.set_source_rgba(
        accent.red().into(),
        accent.green().into(),
        accent.blue().into(),
        accent.alpha().into(),
    );
    context.fill()?;
    context.restore()
}

#[cfg(test)]
pub(crate) fn assert_widgets() {
    let container = Box::new(gtk::Orientation::Horizontal, 4);
    let buttons: Vec<_> = (1..=3).map(crate::workspace_button).collect();
    for button in &buttons {
        container.append(button);
    }
    buttons[0].add_css_class("active");
    let selection = Selection::new(&container);
    let panel = Box::builder().css_name("top-bar").build();
    panel.append(selection.widget());
    let (_, width, _, _) = panel.measure(gtk::Orientation::Horizontal, -1);
    panel.allocate(width, 48, -1, None);
    assert!(!selection.canvas.property::<bool>("can-target"));
    assert!(selection.overlay.is_measure_overlay(&container));
    let first = active_rect(&container, &selection.canvas).unwrap();
    assert_eq!(first.width, 47.0);
    assert_eq!(first.height, 47.0);
    assert_eq!(first.x, 0.0, "highlight has an outer left inset");
    assert_eq!(first.y, 0.0, "highlight has an outer top inset");
    buttons[0].remove_css_class("active");
    buttons[2].add_css_class("empty-active");
    let last = active_rect(&container, &selection.canvas).unwrap();
    assert_eq!(last.x - first.x, 102.0);
    let mut motion = Motion::default();
    motion.advance(Some(first), 0, true);
    motion.advance(Some(last), 1, true);
    assert!(motion.advance(Some(last), 100_001, true));
    let midway = motion.current.unwrap();
    assert!(midway.x > first.x && midway.x < last.x);
    let (fill, accent) = colors(&selection.canvas).unwrap();
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, 48).unwrap();
    {
        let context = cairo::Context::new(&surface).unwrap();
        draw(&context, midway, &fill, &accent).unwrap();
    }
    let stride = surface.stride() as usize;
    let data = surface.data().unwrap();
    let pixel = |x: f64, y: f64| {
        let offset = y as usize * stride + x as usize * 4;
        u32::from_ne_bytes(data[offset..offset + 4].try_into().unwrap())
    };
    assert_eq!(
        pixel(first.x + 18.0, first.y + 18.0),
        0,
        "old highlight stayed behind"
    );
    assert_eq!(
        pixel(midway.x + 18.0, midway.y + 18.0),
        0xffe6bca9,
        "moving fill differs from CSS"
    );
    assert_eq!(
        pixel(midway.x + 18.0, midway.y + midway.height - 1.0),
        0xff8c1612,
        "moving underline differs from CSS"
    );
    assert_eq!(
        pixel(midway.x + 1.0, midway.y),
        0xffe6bca9,
        "top corner leaves a gap at the bar edge"
    );
    assert!(!motion.advance(Some(last), 200_001, true));
    selection.refresh();
    assert!(
        !selection.state.ticking.get(),
        "an unmapped strip must not request frames"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64) -> Rect {
        Rect {
            x,
            y: 5.0,
            width: 36.0,
            height: 36.0,
        }
    }

    #[test]
    fn selection_slides_and_finishes_without_idle_ticks() {
        let mut motion = Motion::default();
        assert!(!motion.advance(Some(rect(0.0)), 0, true));
        assert!(motion.advance(Some(rect(80.0)), 1, true));
        assert_eq!(motion.current, Some(rect(0.0)));
        assert!(motion.advance(Some(rect(80.0)), 100_001, true));
        assert_eq!(motion.current, Some(rect(70.0)));
        assert!(!motion.advance(Some(rect(80.0)), 200_001, true));
        assert_eq!(motion.current, Some(rect(80.0)));
    }

    #[test]
    fn rapid_switches_continue_from_the_visible_position() {
        let mut motion = Motion::default();
        motion.advance(Some(rect(0.0)), 0, true);
        motion.advance(Some(rect(80.0)), 1, true);
        motion.advance(Some(rect(80.0)), 100_001, true);
        assert!(motion.advance(Some(rect(40.0)), 100_002, true));
        assert_eq!(motion.current, Some(rect(70.0)));
        assert!(!motion.advance(Some(rect(40.0)), 300_002, true));
        assert_eq!(motion.current, Some(rect(40.0)));
    }

    #[test]
    fn reduced_motion_snaps_and_no_active_workspace_clears_selection() {
        let mut motion = Motion::default();
        motion.advance(Some(rect(0.0)), 0, true);
        assert!(!motion.advance(Some(rect(80.0)), 1, false));
        assert_eq!(motion.current, Some(rect(80.0)));
        assert!(!motion.advance(Some(rect(80.0)), 2, true));
        assert_eq!(motion.current, Some(rect(80.0)));
        assert!(!motion.advance(None, 3, true));
        assert_eq!(motion.current, None);
        assert!(!motion.advance(Some(rect(40.0)), 4, true));
        assert_eq!(motion.current, Some(rect(40.0)));
    }
}
