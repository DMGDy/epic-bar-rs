use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::{Rc, Weak},
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};

use gtk::{
    gdk, gio, glib, prelude::*, Box, Button, EventControllerMotion, Label, Orientation, Picture,
    Popover,
};

use crate::{
    capture::{self, Job, Outcome, Signals, Target},
    workspaces::{WindowPreview, Workspaces, WORKSPACE_COUNT},
};

const WINDOW_INTERVAL: Duration = Duration::from_secs(1);
const WORKSPACE_INTERVAL: Duration = Duration::from_secs(2);
const WORKSPACE_RETRY_INTERVAL: Duration = Duration::from_millis(250);
const HOVER_FRESHNESS: Duration = Duration::from_millis(500);
const LOCK_INTERVAL: Duration = Duration::from_millis(500);
const CACHE_COUNT: usize = 32;
const CACHE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Anchor {
    Workspace(usize),
    Window(String),
}

struct Info {
    target: Option<Target>,
    source: Option<WindowPreview>,
    title: String,
    detail: String,
}

#[derive(Clone)]
struct Frame {
    texture: gdk::Texture,
    captured: Instant,
    used: u64,
}

#[derive(Default)]
struct Cache {
    frames: BTreeMap<Target, Frame>,
    bytes: usize,
    clock: u64,
}

impl Cache {
    fn get(&mut self, target: &Target) -> Option<Frame> {
        self.clock += 1;
        let frame = self.frames.get_mut(target)?;
        frame.used = self.clock;
        Some(frame.clone())
    }

    fn insert(&mut self, target: Target, mut frame: Frame) {
        self.remove(&target);
        self.clock += 1;
        frame.used = self.clock;
        self.bytes += frame_bytes(&frame);
        self.frames.insert(target, frame);
        while self.frames.len() > CACHE_COUNT || self.bytes > CACHE_BYTES {
            let target = self
                .frames
                .iter()
                .min_by_key(|(_, frame)| frame.used)
                .map(|(target, _)| target.clone())
                .unwrap();
            self.remove(&target);
        }
    }

    fn remove(&mut self, target: &Target) {
        if let Some(frame) = self.frames.remove(target) {
            self.bytes -= frame_bytes(&frame);
        }
    }
}

fn frame_bytes(frame: &Frame) -> usize {
    frame.texture.width() as usize * frame.texture.height() as usize * 4
}

struct State {
    signals: Arc<Signals>,
    info: RefCell<BTreeMap<Anchor, Info>>,
    cache: RefCell<Cache>,
    popups: RefCell<Vec<Weak<Popup>>>,
    selected: RefCell<Option<Anchor>>,
    busy: Cell<bool>,
    private: Cell<bool>,
    last_lock: Cell<Option<Instant>>,
    last_workspaces: Cell<Option<Instant>>,
    last_priority: Cell<Option<Instant>>,
    active_workspace: Cell<Option<usize>>,
    pending_workspace: Cell<Option<usize>>,
    last_window: RefCell<BTreeMap<Target, Instant>>,
    workspace_error: RefCell<Option<String>>,
    lock_error: RefCell<Option<String>>,
    window_errors: RefCell<BTreeMap<Target, String>>,
    last_logged: RefCell<Option<String>>,
}

#[derive(Clone)]
pub struct Previews(Rc<State>);

impl Previews {
    pub fn new(workspaces: &Workspaces) -> Self {
        let previews = Self::empty();
        previews.update(workspaces);
        let weak = Rc::downgrade(&previews.0);
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            state.schedule();
            glib::ControlFlow::Continue
        });
        previews
    }

    fn empty() -> Self {
        Self(Rc::new(State {
            signals: Arc::default(),
            info: RefCell::default(),
            cache: RefCell::default(),
            popups: RefCell::default(),
            selected: RefCell::default(),
            busy: Cell::new(false),
            private: Cell::new(true),
            last_lock: Cell::new(None),
            last_workspaces: Cell::new(None),
            last_priority: Cell::new(None),
            active_workspace: Cell::new(None),
            pending_workspace: Cell::new(None),
            last_window: RefCell::default(),
            workspace_error: RefCell::default(),
            lock_error: RefCell::default(),
            window_errors: RefCell::default(),
            last_logged: RefCell::default(),
        }))
    }

    pub fn signals(&self) -> Arc<Signals> {
        self.0.signals.clone()
    }

    pub fn update(&self, workspaces: &Workspaces) {
        let active = workspaces
            .values()
            .find(|workspace| workspace.active)
            .map(|workspace| workspace.tag);
        if self.0.active_workspace.replace(active) != active {
            for popup in self.0.popups() {
                popup.hide();
            }
            self.0
                .pending_workspace
                .set(active.filter(|tag| (1..=WORKSPACE_COUNT).contains(tag)));
            self.0.last_priority.set(None);
        }
        let mut info = BTreeMap::new();
        for tag in 1..=WORKSPACE_COUNT {
            let detail = workspaces
                .get(&tag)
                .map(|workspace| workspace.tooltip())
                .unwrap_or_else(|| format!("Workspace {tag}\nNo open windows"));
            info.insert(
                Anchor::Workspace(tag),
                Info {
                    target: Some(Target::Workspace(tag)),
                    source: None,
                    title: format!("Workspace {tag}"),
                    detail: detail.split_once('\n').unwrap().1.to_string(),
                },
            );
        }
        for window in workspaces.values().flat_map(|workspace| &workspace.windows) {
            info.insert(
                Anchor::Window(window.address.clone()),
                Info {
                    target: window
                        .preview
                        .as_ref()
                        .map(|source| Target::Window(source.identifier.clone())),
                    source: window.preview.clone(),
                    title: window.caption().to_string(),
                    detail: if window.urgent {
                        "Needs attention".into()
                    } else {
                        format!("Workspace {}", window.tag)
                    },
                },
            );
        }
        let valid = |target: &Target| {
            info.values()
                .any(|value| value.target.as_ref() == Some(target))
        };
        let stale: Vec<_> = self
            .0
            .cache
            .borrow()
            .frames
            .keys()
            .filter(|target| !valid(target))
            .cloned()
            .collect();
        for target in stale {
            self.0.cache.borrow_mut().remove(&target);
        }
        self.0
            .last_window
            .borrow_mut()
            .retain(|target, _| valid(target));
        self.0
            .window_errors
            .borrow_mut()
            .retain(|target, _| valid(target));
        *self.0.info.borrow_mut() = info;
        for popup in self.0.popups() {
            if self.0.info.borrow().contains_key(&popup.anchor) {
                popup.refresh();
            } else {
                popup.hide();
            }
        }
    }

    pub fn attach(&self, button: &Button, anchor: Anchor) -> HoverPreview {
        let title = Label::builder()
            .css_classes(["popup-heading"])
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(38)
            .lines(2)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let picture = Picture::builder()
            .can_shrink(true)
            .content_fit(gtk::ContentFit::Contain)
            .width_request(320)
            .height_request(180)
            .css_classes(["preview-image"])
            .build();
        let status = Label::builder()
            .css_classes(["popup-secondary"])
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(38)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .build();
        let detail = Label::builder()
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(38)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .build();
        let details = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .max_content_height(120)
            .child(&detail)
            .build();
        let retry = Button::builder()
            .label("Retry capture")
            .focus_on_click(false)
            .focusable(false)
            .halign(gtk::Align::Start)
            .visible(false)
            .build();
        let body = Box::new(Orientation::Vertical, 8);
        body.append(&title);
        body.append(&picture);
        body.append(&status);
        body.append(&details);
        body.append(&retry);
        let popover = Popover::builder()
            .css_classes(["bar-popover", "hover-preview"])
            .autohide(false)
            .can_focus(false)
            .focusable(false)
            .has_arrow(true)
            .position(match anchor {
                Anchor::Workspace(_) => gtk::PositionType::Bottom,
                Anchor::Window(_) => gtk::PositionType::Top,
            })
            .child(&body)
            .build();
        popover.set_parent(button);
        button.add_css_class("preview-anchor");
        button.set_has_tooltip(false);
        let popup = Rc::new(Popup {
            state: self.0.clone(),
            anchor,
            parent: button.downgrade(),
            popover,
            title,
            picture,
            status,
            detail,
            retry,
            show_timer: RefCell::default(),
            hide_timer: RefCell::default(),
            show_deadline: Cell::new(None),
        });
        self.0.popups.borrow_mut().push(Rc::downgrade(&popup));
        if matches!(popup.anchor, Anchor::Workspace(_)) {
            let click = gtk::GestureClick::new();
            click.set_button(1);
            let weak = Rc::downgrade(&popup);
            click.connect_released(move |click, count, x, y| {
                if count != 1 {
                    return;
                }
                if let (Some(popup), Some(body)) = (weak.upgrade(), click.widget()) {
                    if body.contains(x, y)
                        && !Popup::is_preview_control(
                            body.pick(x, y, gtk::PickFlags::DEFAULT),
                            &body,
                        )
                    {
                        click.set_state(gtk::EventSequenceState::Claimed);
                        popup.activate_workspace();
                    }
                }
            });
            body.add_controller(click);
        }
        let motion = EventControllerMotion::new();
        let enter = Rc::downgrade(&popup);
        motion.connect_enter(move |_, _, _| {
            if let Some(popup) = enter.upgrade() {
                popup.enter(true);
            }
        });
        let leave = Rc::downgrade(&popup);
        motion.connect_leave(move |_| {
            if let Some(popup) = leave.upgrade() {
                popup.leave();
            }
        });
        button.add_controller(motion);
        let motion = EventControllerMotion::new();
        let enter = Rc::downgrade(&popup);
        motion.connect_enter(move |_, _, _| {
            if let Some(popup) = enter.upgrade() {
                popup.enter(false);
            }
        });
        let leave = Rc::downgrade(&popup);
        motion.connect_leave(move |_| {
            if let Some(popup) = leave.upgrade() {
                popup.leave();
            }
        });
        popup.popover.add_controller(motion);
        let weak = Rc::downgrade(&popup);
        popup.popover.connect_unmap(move |_| {
            if let Some(popup) = weak.upgrade() {
                popup.state.sync_open();
            }
        });
        let hide = Rc::downgrade(&popup);
        button.connect_unmap(move |_| {
            if let Some(popup) = hide.upgrade() {
                popup.hide();
            }
        });
        let hide = Rc::downgrade(&popup);
        button.connect_clicked(move |_| {
            if let Some(popup) = hide.upgrade() {
                popup.hide();
            }
        });
        let weak = Rc::downgrade(&popup);
        popup.retry.connect_clicked(move |_| {
            if let Some(popup) = weak.upgrade() {
                if matches!(popup.anchor, Anchor::Workspace(_)) {
                    popup.hide();
                } else if let Some(target) = popup
                    .state
                    .info
                    .borrow()
                    .get(&popup.anchor)
                    .and_then(|info| info.target.as_ref())
                {
                    popup.state.last_window.borrow_mut().remove(target);
                }
                popup.state.workspace_error.borrow_mut().take();
                popup.state.lock_error.borrow_mut().take();
                popup.state.window_errors.borrow_mut().clear();
                popup.state.last_workspaces.set(None);
                popup.state.last_priority.set(None);
                popup.state.last_lock.set(None);
                popup.refresh();
                popup.state.schedule();
            }
        });
        HoverPreview {
            parent: button.clone(),
            popup,
        }
    }
}

impl State {
    fn popups(&self) -> Vec<Rc<Popup>> {
        let mut popups = self.popups.borrow_mut();
        popups.retain(|popup| popup.strong_count() > 0);
        popups.iter().filter_map(Weak::upgrade).collect()
    }

    fn sync_open(&self) {
        let open = self.popups().iter().any(|popup| popup.popover.is_mapped());
        if self.signals.popup_open.swap(open, Ordering::AcqRel) != open {
            self.signals.invalidate();
        }
    }

    fn schedule(self: &Rc<Self>) {
        if !self.signals.connected.load(Ordering::Acquire) {
            if !self.private.get() {
                self.finish(
                    &Job::Lock,
                    Err("Hyprland event stream disconnected; previews cleared".into()),
                );
            }
            return;
        }
        if self.busy.get() {
            return;
        }
        let now = Instant::now();
        let Some(job) = self.next_job(now) else {
            return;
        };
        match &job {
            Job::Window(source) => {
                self.last_window
                    .borrow_mut()
                    .insert(Target::Window(source.identifier.clone()), now);
            }
            Job::Workspaces => self.last_workspaces.set(Some(now)),
            Job::Workspace(_) => self.last_priority.set(Some(now)),
            _ => {}
        }
        let worker_job = job.clone();
        self.start(job, move |signals, epoch| {
            capture::run(&worker_job, signals, epoch)
        });
    }

    fn next_job(&self, now: Instant) -> Option<Job> {
        if self.private.get() {
            return due(self.last_lock.get(), LOCK_INTERVAL, now).then_some(Job::Lock);
        }
        let workspace_ready = !self.signals.popup_open.load(Ordering::Acquire)
            && self.signals.settled()
            && self.workspace_error.borrow().is_none();
        if workspace_ready {
            if let Some(tag) = self
                .pending_workspace
                .get()
                .filter(|_| due(self.last_priority.get(), WORKSPACE_RETRY_INTERVAL, now))
            {
                return Some(Job::Workspace(tag));
            }
        }
        let window = self.selected.borrow().as_ref().and_then(|anchor| {
            let info = self.info.borrow();
            let info = info.get(anchor)?;
            let source = info.source.as_ref()?;
            let target = info.target.as_ref()?;
            (!self.window_errors.borrow().contains_key(target)
                && due(
                    self.last_window.borrow().get(target).copied(),
                    WINDOW_INTERVAL,
                    now,
                ))
            .then(|| source.clone())
        });
        Some(if let Some(source) = window {
            Job::Window(source)
        } else if workspace_ready && due(self.last_workspaces.get(), WORKSPACE_INTERVAL, now) {
            Job::Workspaces
        } else if due(self.last_lock.get(), LOCK_INTERVAL, now) {
            Job::Lock
        } else {
            return None;
        })
    }

    fn start(
        self: &Rc<Self>,
        job: Job,
        work: impl FnOnce(&Signals, u64) -> Result<Outcome, String> + Send + 'static,
    ) {
        self.busy.set(true);
        let signals = self.signals.clone();
        let epoch = signals.epoch.load(Ordering::Acquire);
        let weak = Rc::downgrade(self);
        let worker = gio::spawn_blocking(move || work(&signals, epoch));
        glib::spawn_future_local(async move {
            let result = match worker.await {
                Ok(result) => result,
                Err(_) => Err("preview capture worker panicked".into()),
            };
            if let Some(state) = weak.upgrade() {
                state.busy.set(false);
                if matches!(result, Ok(Outcome::Locked)) || state.signals.current(epoch) {
                    state.finish(&job, result);
                }
                state.schedule();
            }
        });
    }

    fn finish(&self, job: &Job, result: Result<Outcome, String>) {
        let result = result.and_then(|outcome| {
            if let Outcome::Images(images) = outcome {
                let frames = images
                    .into_iter()
                    .map(|image| {
                        capture::png_dimensions(&image.png)?;
                        let texture = gdk::Texture::from_bytes(&glib::Bytes::from_owned(image.png))
                            .map_err(|error| format!("cannot decode preview: {error}"))?;
                        Ok((
                            image.target,
                            Frame {
                                texture,
                                captured: Instant::now(),
                                used: 0,
                            },
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                for (target, frame) in frames {
                    if self
                        .info
                        .borrow()
                        .values()
                        .any(|info| info.target.as_ref() == Some(&target))
                    {
                        if matches!(target, Target::Workspace(tag) if self.pending_workspace.get() == Some(tag)) {
                            self.pending_workspace.set(None);
                        }
                        self.cache.borrow_mut().insert(target, frame);
                    }
                }
                Ok(Outcome::Unlocked)
            } else {
                Ok(outcome)
            }
        });
        match result {
            Ok(Outcome::Locked) => {
                if !self.private.replace(true) {
                    self.signals.invalidate();
                }
                *self.cache.borrow_mut() = Cache::default();
                for popup in self.popups() {
                    popup.hide();
                }
                self.last_lock.set(Some(Instant::now()));
            }
            Ok(Outcome::Unlocked) => {
                if self.private.replace(false)
                    && !matches!(job, Job::Workspaces | Job::Workspace(_))
                {
                    self.last_workspaces.set(None);
                }
                self.last_lock.set(Some(Instant::now()));
                self.lock_error.borrow_mut().take();
            }
            Ok(Outcome::Skipped) => {}
            Ok(Outcome::Images(_)) => unreachable!(),
            Err(error) => {
                self.private.set(true);
                *self.cache.borrow_mut() = Cache::default();
                if self.last_logged.borrow().as_ref() != Some(&error) {
                    eprintln!("epic-bar-rs: {error}");
                    *self.last_logged.borrow_mut() = Some(error.clone());
                }
                match job {
                    Job::Workspaces | Job::Workspace(_) => {
                        *self.workspace_error.borrow_mut() = Some(error)
                    }
                    Job::Window(source) => {
                        *self.workspace_error.borrow_mut() = Some(error.clone());
                        self.window_errors
                            .borrow_mut()
                            .insert(Target::Window(source.identifier.clone()), error);
                    }
                    Job::Lock => {
                        *self.lock_error.borrow_mut() = Some(error);
                        self.last_lock.set(Some(Instant::now()));
                    }
                }
            }
        }
        for popup in self.popups() {
            popup.refresh();
        }
    }
}

fn due(last: Option<Instant>, interval: Duration, now: Instant) -> bool {
    last.is_none_or(|last| now.duration_since(last) >= interval)
}

struct Popup {
    state: Rc<State>,
    anchor: Anchor,
    parent: glib::WeakRef<Button>,
    popover: Popover,
    title: Label,
    picture: Picture,
    status: Label,
    detail: Label,
    retry: Button,
    show_timer: RefCell<Option<glib::SourceId>>,
    hide_timer: RefCell<Option<glib::SourceId>>,
    show_deadline: Cell<Option<Instant>>,
}

impl Popup {
    fn enter(self: &Rc<Self>, delay: bool) {
        cancel(&self.hide_timer);
        if delay {
            for popup in self.state.popups() {
                if popup.anchor != self.anchor {
                    popup.hide();
                }
            }
            if let Anchor::Workspace(tag) = self.anchor {
                if !self.popover.is_visible() && self.state.active_workspace.get() == Some(tag) {
                    let fresh = self
                        .state
                        .cache
                        .borrow()
                        .frames
                        .get(&Target::Workspace(tag))
                        .is_some_and(|frame| frame.captured.elapsed() < HOVER_FRESHNESS);
                    if !fresh {
                        self.state.pending_workspace.set(Some(tag));
                        self.state.last_priority.set(None);
                    }
                }
            }
        }
        *self.state.selected.borrow_mut() = Some(self.anchor.clone());
        self.state.schedule();
        if !delay || self.popover.is_visible() || self.show_timer.borrow().is_some() {
            return;
        }
        self.show_deadline
            .set(Some(Instant::now() + Duration::from_secs(1)));
        let weak = Rc::downgrade(self);
        *self.show_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            Duration::from_millis(400),
            move || {
                if let Some(popup) = weak.upgrade() {
                    popup.show_timer.borrow_mut().take();
                    popup.show();
                }
            },
        ));
    }

    fn leave(self: &Rc<Self>) {
        cancel(&self.show_timer);
        cancel(&self.hide_timer);
        let weak = Rc::downgrade(self);
        *self.hide_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            Duration::from_millis(150),
            move || {
                if let Some(popup) = weak.upgrade() {
                    popup.hide_timer.borrow_mut().take();
                    popup.hide();
                }
            },
        ));
    }

    fn show(self: &Rc<Self>) {
        if !self
            .parent
            .upgrade()
            .is_some_and(|button| button.is_mapped())
        {
            return;
        }
        if matches!(self.anchor, Anchor::Workspace(_))
            && self.state.pending_workspace.get().is_some()
            && self.state.workspace_error.borrow().is_none()
            && self.state.lock_error.borrow().is_none()
            && self.state.signals.connected.load(Ordering::Acquire)
            && self
                .show_deadline
                .get()
                .is_some_and(|deadline| Instant::now() < deadline)
        {
            self.state.schedule();
            let weak = Rc::downgrade(self);
            *self.show_timer.borrow_mut() = Some(glib::timeout_add_local_once(
                Duration::from_millis(50),
                move || {
                    if let Some(popup) = weak.upgrade() {
                        popup.show_timer.borrow_mut().take();
                        popup.show();
                    }
                },
            ));
            return;
        }
        for popup in self.state.popups() {
            if popup.anchor != self.anchor {
                popup.hide();
            }
        }
        self.state.signals.popup_open.store(true, Ordering::Release);
        self.state.signals.invalidate();
        self.refresh_content();
        self.popover.popup();
    }

    fn hide(&self) {
        cancel(&self.show_timer);
        cancel(&self.hide_timer);
        self.show_deadline.set(None);
        self.popover.popdown();
        self.picture.set_paintable(None::<&gdk::Paintable>);
        if self.state.selected.borrow().as_ref() == Some(&self.anchor) {
            self.state.selected.borrow_mut().take();
        }
        self.state.sync_open();
    }

    fn activate_workspace(&self) {
        if matches!(self.anchor, Anchor::Workspace(_)) {
            self.hide();
            if let Some(button) = self.parent.upgrade() {
                button.emit_clicked();
            }
        }
    }

    fn refresh(&self) {
        if self.popover.is_visible() {
            self.refresh_content();
        }
    }

    fn is_preview_control(mut target: Option<gtk::Widget>, body: &gtk::Widget) -> bool {
        while let Some(widget) = target {
            if &widget == body {
                break;
            }
            if widget.is::<Button>() || widget.is::<gtk::Scrollbar>() || widget.is::<gtk::Range>() {
                return true;
            }
            target = widget.parent();
        }
        false
    }

    fn refresh_content(&self) {
        let info = self.state.info.borrow();
        let Some(info) = info.get(&self.anchor) else {
            return;
        };
        self.title.set_text(&info.title);
        self.title.set_tooltip_text(Some(&info.title));
        self.detail.set_text(&info.detail);
        let error = info
            .target
            .as_ref()
            .and_then(|target| match target {
                Target::Workspace(_) => self.state.workspace_error.borrow().clone(),
                Target::Window(_) => self.state.window_errors.borrow().get(target).cloned(),
            })
            .or_else(|| self.state.lock_error.borrow().clone());
        self.retry.set_visible(error.is_some());
        self.picture.set_paintable(None::<&gdk::Paintable>);
        if let Some(error) = error {
            self.status.set_text(&error);
        } else if self.state.private.get() {
            self.status.set_text("Waiting for an unlocked session");
        } else if let Some(target) = &info.target {
            if let Some(frame) = self.state.cache.borrow_mut().get(target) {
                self.picture.set_paintable(Some(&frame.texture));
                let age = frame.captured.elapsed().as_secs();
                self.status.set_text(&match target {
                    Target::Workspace(_) => format!("Last seen {age}s ago"),
                    Target::Window(_) => format!("Window snapshot - {age}s ago"),
                });
            } else {
                self.status.set_text(match target {
                    Target::Workspace(_) => {
                        "No snapshot yet. Captured while visible, with previews closed."
                    }
                    Target::Window(_) => "Capturing window...",
                });
            }
        } else {
            self.status.set_text(
                "Window preview unavailable: Hyprland must provide stableId and window size.",
            );
        }
    }
}

fn cancel(timer: &RefCell<Option<glib::SourceId>>) {
    if let Some(timer) = timer.borrow_mut().take() {
        timer.remove();
    }
}

pub struct HoverPreview {
    parent: Button,
    popup: Rc<Popup>,
}

impl HoverPreview {
    pub fn hide(&self) {
        self.popup.hide();
    }
}

impl Drop for HoverPreview {
    fn drop(&mut self) {
        self.popup.hide();
        self.popup.popover.unparent();
        self.parent.remove_css_class("preview-anchor");
        self.parent
            .set_has_tooltip(self.parent.tooltip_text().is_some());
    }
}

#[cfg(test)]
pub(crate) fn assert_widgets() {
    let previews = Previews::empty();
    previews.update(&Workspaces::new());
    let button = Button::with_label("1");
    let before = button.measure(Orientation::Horizontal, -1).1;
    let preview = previews.attach(&button, Anchor::Workspace(1));
    assert_eq!(button.measure(Orientation::Horizontal, -1).1, before);
    assert!(!preview.popup.popover.is_autohide());
    assert!(!preview.popup.popover.is_focusable());
    assert_eq!(preview.popup.popover.position(), gtk::PositionType::Bottom);
    assert_eq!(preview.popup.picture.width_request(), 320);
    assert!(!button.has_tooltip());
    let pixels =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 8, 4).unwrap();
    pixels.fill(0xaabbccff);
    let texture = gdk::Texture::for_pixbuf(&pixels);
    let frame = || Frame {
        texture: texture.clone(),
        captured: Instant::now(),
        used: 0,
    };
    let mut cache = Cache::default();
    for id in 0..CACHE_COUNT {
        cache.insert(Target::Workspace(id), frame());
    }
    cache.get(&Target::Workspace(0)).unwrap();
    cache.insert(Target::Workspace(CACHE_COUNT), frame());
    assert!(cache.get(&Target::Workspace(1)).is_none());
    assert_eq!(cache.frames.len(), CACHE_COUNT);
    assert_eq!(cache.bytes, CACHE_COUNT * 8 * 4 * 4);
    let large = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 1024, 1024)
        .unwrap();
    large.fill(0xaabbccff);
    let large_texture = gdk::Texture::for_pixbuf(&large);
    let mut budget = Cache::default();
    for id in 0..9 {
        budget.insert(
            Target::Workspace(id),
            Frame {
                texture: large_texture.clone(),
                captured: Instant::now(),
                used: 0,
            },
        );
    }
    assert_eq!(budget.bytes, CACHE_BYTES);
    assert_eq!(budget.frames.len(), 8);
    let png = pixels.save_to_bufferv("png", &[]).unwrap();
    previews.0.pending_workspace.set(Some(2));
    previews
        .0
        .finish(&Job::Workspace(2), Ok(Outcome::Images(vec![])));
    assert_eq!(previews.0.pending_workspace.get(), Some(2));
    previews.0.finish(
        &Job::Workspace(1),
        Ok(Outcome::Images(vec![capture::Image {
            target: Target::Workspace(1),
            png,
        }])),
    );
    assert_eq!(
        previews.0.pending_workspace.get(),
        Some(2),
        "a different snapshot must not acknowledge a newer visit"
    );
    preview.popup.refresh_content();
    assert!(preview.popup.picture.paintable().is_some());
    assert!(preview.popup.status.text().starts_with("Last seen "));
    assert_mapped_popup(&button, &preview, before);
    previews.0.finish(&Job::Lock, Ok(Outcome::Locked));
    assert!(previews.0.cache.borrow().frames.is_empty());
    assert!(preview.popup.picture.paintable().is_none());
    assert!(previews.0.private.get());
    preview
        .popup
        .show_timer
        .borrow_mut()
        .replace(glib::timeout_add_local_once(Duration::from_secs(5), || {
            panic!("cancelled hover timer ran")
        }));
    preview.hide();
    assert!(preview.popup.show_timer.borrow().is_none());
    let weak = Rc::downgrade(&preview.popup);
    drop(preview);
    assert!(
        weak.upgrade().is_none(),
        "hover handlers must not retain closed previews"
    );
    assert!(previews.0.popups().is_empty());
    assert!(button
        .last_child()
        .is_some_and(|child| !child.is::<Popover>()));
    assert_window_identity(&texture);
    assert_async_worker();
}

#[cfg(test)]
fn assert_async_worker() {
    let previews = Previews::empty();
    previews.update(&Workspaces::new());
    previews.0.signals.connected.store(true, Ordering::Release);
    *previews.0.workspace_error.borrow_mut() = Some("Automatic captures disabled for test".into());
    let (started, ready) = async_channel::bounded(1);
    let (release, wait) = std::sync::mpsc::channel();
    previews.0.start(Job::Lock, move |_, _| {
        started.send_blocking(()).unwrap();
        wait.recv_timeout(Duration::from_secs(2))
            .map_err(|error| error.to_string())?;
        Ok(Outcome::Unlocked)
    });
    glib::MainContext::default().block_on(async {
        ready.recv().await.unwrap();
        glib::timeout_future(Duration::from_millis(10)).await;
        assert!(
            previews.0.busy.get(),
            "GTK must run while the capture is blocked"
        );
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while previews.0.busy.get() && Instant::now() < deadline {
            glib::timeout_future(Duration::from_millis(5)).await;
        }
        assert!(!previews.0.busy.get());
        assert!(!previews.0.private.get());
    });
    let (started, ready) = async_channel::bounded(1);
    let (release, wait) = std::sync::mpsc::channel();
    let pixels =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 8, 4).unwrap();
    pixels.fill(0xaabbccff);
    let png = pixels.save_to_bufferv("png", &[]).unwrap();
    previews.0.start(Job::Workspace(1), move |_, _| {
        started.send_blocking(()).unwrap();
        wait.recv_timeout(Duration::from_secs(2))
            .map_err(|error| error.to_string())?;
        Ok(Outcome::Images(vec![capture::Image {
            target: Target::Workspace(1),
            png,
        }]))
    });
    glib::MainContext::default().block_on(async {
        ready.recv().await.unwrap();
        previews.0.pending_workspace.set(Some(2));
        previews
            .0
            .signals
            .observe(&crate::workspaces::HyprEvent::Workspace);
        previews
            .0
            .signals
            .observe(&crate::workspaces::HyprEvent::Workspace);
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while previews.0.busy.get() && Instant::now() < deadline {
            glib::timeout_future(Duration::from_millis(5)).await;
        }
        assert!(!previews.0.busy.get());
        assert!(
            previews.0.cache.borrow().frames.is_empty(),
            "late replies must not populate the cache after switching away and back"
        );
        assert_eq!(previews.0.pending_workspace.get(), Some(2));
    });
}

#[cfg(test)]
fn assert_window_identity(texture: &gdk::Texture) {
    use crate::workspaces::{Window, Workspace};

    let mut workspaces = Workspaces::from([(
        1,
        Workspace {
            tag: 1,
            order: 0,
            active: true,
            windows: vec![Window {
                name: "Fixture".into(),
                info: "Live title".into(),
                address: "abc".into(),
                class: "kitty".into(),
                tag: 1,
                order: 0,
                focused: true,
                urgent: false,
                preview: Some(WindowPreview {
                    identifier: "ab".into(),
                    width: 1280,
                    height: 720,
                    monitor: 1,
                }),
            }],
        },
    )]);
    let previews = Previews::empty();
    previews.update(&workspaces);
    let tasks = crate::tasks::Tasks::new(&workspaces, Some(previews.clone()));
    let button = tasks
        .widget()
        .first_child()
        .unwrap()
        .next_sibling()
        .unwrap()
        .downcast::<Button>()
        .unwrap();
    assert!(button.has_css_class("preview-anchor"));
    assert!(!button.has_tooltip());
    let popup = Rc::downgrade(&previews.0.popups()[0]);
    assert_eq!(
        popup.upgrade().unwrap().popover.position(),
        gtk::PositionType::Top
    );
    previews.0.cache.borrow_mut().insert(
        Target::Window("ab".into()),
        Frame {
            texture: texture.clone(),
            captured: Instant::now(),
            used: 0,
        },
    );
    workspaces.get_mut(&1).unwrap().windows[0]
        .preview
        .as_mut()
        .unwrap()
        .identifier = "cd".into();
    previews.update(&workspaces);
    tasks.update(&workspaces);
    assert!(previews
        .0
        .cache
        .borrow_mut()
        .get(&Target::Window("ab".into()))
        .is_none());
    assert_eq!(
        button,
        tasks
            .widget()
            .first_child()
            .unwrap()
            .next_sibling()
            .unwrap()
            .downcast::<Button>()
            .unwrap()
    );
    assert_eq!(
        previews.0.info.borrow()[&Anchor::Window("abc".into())].target,
        Some(Target::Window("cd".into()))
    );
    drop(tasks);
    assert!(popup.upgrade().is_none());
}

#[cfg(test)]
fn assert_mapped_popup(button: &Button, preview: &HoverPreview, natural_width: i32) {
    use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};

    let root = gtk::Window::builder()
        .default_width(64)
        .default_height(48)
        .opacity(0.0)
        .child(button)
        .build();
    LayerShell::init_layer_shell(&root);
    LayerShell::set_namespace(&root, Some("epic-bar-preview-test"));
    LayerShell::set_layer(&root, Layer::Background);
    LayerShell::set_keyboard_mode(&root, KeyboardMode::None);
    preview.popup.popover.set_opacity(0.0);
    root.set_visible(true);
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(2);
    while (!button.is_mapped() || button.width() == 0) && Instant::now() < deadline {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(button.is_mapped());
    let focus = gtk::prelude::GtkWindowExt::focus(&root);
    let state = &preview.popup.state;
    state.active_workspace.set(Some(1));
    state
        .cache
        .borrow_mut()
        .frames
        .get_mut(&Target::Workspace(1))
        .unwrap()
        .captured = Instant::now() - Duration::from_secs(1);
    state.signals.connected.store(true, Ordering::Release);
    state.busy.set(true);
    preview.popup.enter(true);
    assert_eq!(
        state.pending_workspace.get(),
        Some(1),
        "hovering a stale active workspace must request a fresh frame"
    );
    cancel(&preview.popup.show_timer);
    let epoch = state.signals.epoch.load(Ordering::Acquire);
    preview.popup.show();
    assert!(
        !preview.popup.popover.is_visible(),
        "the popup must not block its requested capture"
    );
    assert_eq!(state.signals.epoch.load(Ordering::Acquire), epoch);
    assert!(preview.popup.show_timer.borrow().is_some());
    let texture = state.cache.borrow().frames[&Target::Workspace(1)]
        .texture
        .clone();
    state.finish(
        &Job::Workspace(1),
        Ok(Outcome::Images(vec![capture::Image {
            target: Target::Workspace(1),
            png: texture.save_to_png_bytes().to_vec(),
        }])),
    );
    assert_eq!(state.pending_workspace.get(), None);
    let deadline = Instant::now() + Duration::from_secs(2);
    while preview.popup.popover.width() < 320 && Instant::now() < deadline {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(preview.popup.popover.is_mapped());
    assert!(preview.popup.popover.width() >= 320);
    assert!(preview.popup.popover.height() >= 180);
    assert_eq!(button.measure(Orientation::Horizontal, -1).1, natural_width);
    assert_eq!(gtk::prelude::GtkWindowExt::focus(&root), focus);
    assert_eq!(LayerShell::keyboard_mode(&root), KeyboardMode::None);
    assert!(!preview.popup.popover.can_focus());
    assert!(preview
        .popup
        .state
        .signals
        .popup_open
        .load(Ordering::Acquire));
    let clicks = Rc::new(Cell::new(0));
    let count = clicks.clone();
    button.connect_clicked(move |_| count.set(count.get() + 1));
    let body = preview.popup.popover.child().unwrap();
    let controllers = body.observe_controllers();
    let click = (0..controllers.n_items())
        .find_map(|index| {
            controllers
                .item(index)?
                .downcast::<gtk::GestureClick>()
                .ok()
        })
        .unwrap();
    let emit_click = |widget: &gtk::Widget| {
        let center = widget.compute_bounds(&body).unwrap().center();
        click.emit_by_name::<()>(
            "released",
            &[&1i32, &f64::from(center.x()), &f64::from(center.y())],
        );
    };
    preview.popup.retry.set_visible(true);
    let deadline = Instant::now() + Duration::from_secs(2);
    while preview.popup.retry.height() == 0 && Instant::now() < deadline {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(preview.popup.retry.height() > 0);
    emit_click(preview.popup.retry.upcast_ref());
    assert_eq!(clicks.get(), 0, "Retry must not switch workspaces");
    let scrollbar = gtk::Scrollbar::new(Orientation::Vertical, None::<&gtk::Adjustment>);
    assert!(Popup::is_preview_control(Some(scrollbar.upcast()), &body));
    emit_click(preview.popup.picture.upcast_ref());
    assert_eq!(
        clicks.get(),
        1,
        "clicking the image must activate its workspace exactly once"
    );
    assert!(!preview.popup.popover.is_visible());
    preview.popup.show();
    emit_click(preview.popup.title.upcast_ref());
    assert_eq!(
        clicks.get(),
        2,
        "clicking the preview title must also activate its workspace"
    );
    assert_eq!(gtk::prelude::GtkWindowExt::focus(&root), focus);
    state.pending_workspace.set(Some(1));
    preview
        .popup
        .show_deadline
        .set(Some(Instant::now() - Duration::from_millis(1)));
    preview.popup.show();
    assert!(
        preview.popup.popover.is_visible(),
        "freshness waits must have a display deadline"
    );
    let other_button = Button::with_label("2");
    let other = Previews(state.clone()).attach(&other_button, Anchor::Workspace(2));
    other.popup.enter(true);
    assert!(
        !preview.popup.popover.is_visible(),
        "entering another source must unblock monitor capture immediately"
    );
    other.hide();
    preview.hide();
    state.signals.connected.store(false, Ordering::Release);
    state.busy.set(false);
    root.set_visible(false);
    root.set_child(None::<&gtk::Widget>);
    root.close();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspaces::Workspace;

    #[test]
    fn visits_bypass_idle_sampling_and_coalesce_to_the_latest_workspace() {
        let previews = Previews::empty();
        let now = Instant::now();
        previews.0.private.set(false);
        previews.0.last_workspaces.set(Some(now));
        previews.0.last_lock.set(Some(now));
        let visit = |tag| {
            previews.update(&Workspaces::from([(
                tag,
                Workspace {
                    tag,
                    order: 0,
                    active: true,
                    windows: vec![],
                },
            )]))
        };
        visit(1);
        assert!(matches!(previews.0.next_job(now), Some(Job::Workspace(1))));
        previews.0.last_priority.set(Some(now));
        assert!(
            previews.0.next_job(now).is_none(),
            "skipped captures must not spin"
        );
        assert!(matches!(
            previews.0.next_job(now + WORKSPACE_RETRY_INTERVAL),
            Some(Job::Workspace(1))
        ));
        visit(2);
        assert!(matches!(previews.0.next_job(now), Some(Job::Workspace(2))));
        visit(1);
        assert_eq!(previews.0.pending_workspace.get(), Some(1));
        previews.0.pending_workspace.set(None);
        visit(1);
        assert!(
            previews.0.next_job(now).is_none(),
            "unchanged metadata must not requeue a completed visit"
        );
        previews.0.signals.popup_open.store(true, Ordering::Release);
        visit(2);
        assert!(
            previews.0.next_job(now).is_none(),
            "open previews must still prevent monitor capture"
        );
    }

    #[test]
    fn locked_sessions_only_schedule_bounded_lock_checks() {
        let previews = Previews::empty();
        let now = Instant::now();
        previews.0.pending_workspace.set(Some(1));
        previews.0.last_lock.set(Some(now));
        assert!(previews.0.next_job(now).is_none());
        assert!(matches!(
            previews.0.next_job(now + LOCK_INTERVAL),
            Some(Job::Lock)
        ));
    }
}
