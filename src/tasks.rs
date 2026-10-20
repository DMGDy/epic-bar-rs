use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*, Box, Button, Fixed, Image, Label, Orientation, Widget};

use crate::{
    selection::{Motion, Rect},
    workspaces::{self, Window, Workspaces},
};

const HEIGHT: i32 = 37;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Tag(usize),
    Window(String),
    Empty(usize),
}

enum Item<'a> {
    Tag(usize),
    Window(&'a Window, bool),
    Empty(usize),
}

impl Item<'_> {
    fn key(&self) -> Key {
        match self {
            Self::Tag(tag) => Key::Tag(*tag),
            Self::Window(window, _) => Key::Window(window.address.clone()),
            Self::Empty(tag) => Key::Empty(*tag),
        }
    }

    fn create(&self) -> Tile {
        match self {
            Self::Tag(tag) => Tile::Tag(
                Label::builder()
                    .css_name("tag-label")
                    .name(tag.to_string())
                    .label(tag.to_string())
                    .build(),
            ),
            Self::Empty(_) => Tile::Empty(
                Button::builder()
                    .css_name("window-box-empty")
                    .css_classes(["bar-button"])
                    .label("󰟢")
                    .build(),
            ),
            Self::Window(window, _) => Tile::Window(WindowTile::new(window)),
        }
    }
}

struct WindowTile {
    button: Button,
    label: Label,
    icon: Image,
    class: String,
}

impl WindowTile {
    fn new(window: &Window) -> Self {
        let button = Button::builder()
            .css_name("window-box")
            .css_classes(["bar-button"])
            .name(&window.address)
            .build();
        let contents = Box::new(Orientation::Horizontal, 6);
        let icon = Image::builder()
            .file(format!("assets/apps/{}.svg", window.class))
            .css_name("icon-image")
            .pixel_size(18)
            .build();
        let label = crate::task_label(&window.name);
        contents.append(&icon);
        contents.append(&label);
        button.set_child(Some(&contents));
        let address = window.address.clone();
        button.connect_clicked(move |_| {
            let address = address.clone();
            gio::spawn_blocking(move || workspaces::switch_window(&address));
        });
        Self {
            button,
            label,
            icon,
            class: window.class.clone(),
        }
    }

    fn update(&mut self, window: &Window, active: bool) {
        self.label.set_text(&window.name);
        self.button.set_tooltip_text(Some(&window.info));
        if active {
            self.button.add_css_class("active");
        } else {
            self.button.remove_css_class("active");
        }
        if self.class != window.class {
            self.icon
                .set_from_file(Some(format!("assets/apps/{}.svg", window.class)));
            self.class.clone_from(&window.class);
        }
    }
}

enum Tile {
    Tag(Label),
    Window(WindowTile),
    Empty(Button),
}

impl Tile {
    fn widget(&self) -> &Widget {
        match self {
            Self::Tag(label) => label.upcast_ref(),
            Self::Window(window) => window.button.upcast_ref(),
            Self::Empty(button) => button.upcast_ref(),
        }
    }
}

struct Entry {
    tile: Tile,
    motion: Motion,
    target: Rect,
}

#[derive(Default)]
struct State {
    entries: BTreeMap<Key, Entry>,
    ticking: bool,
}

pub struct Tasks {
    fixed: Fixed,
    state: Rc<RefCell<State>>,
}

impl Tasks {
    pub fn new(workspaces: &Workspaces) -> Self {
        let fixed = Fixed::builder()
            .css_name("workspace-windows-container")
            .hexpand(true)
            .vexpand(true)
            .build();
        let state = Rc::new(RefCell::new(State::default()));
        let map_state = state.clone();
        fixed.connect_map(move |fixed| start(fixed, &map_state));
        let tasks = Self { fixed, state };
        tasks.update(workspaces);
        tasks
    }

    pub fn widget(&self) -> &Fixed {
        &self.fixed
    }

    pub fn update(&self, workspaces: &Workspaces) {
        let animate = self.fixed.is_mapped()
            && self
                .fixed
                .settings()
                .property::<bool>("gtk-enable-animations");
        if self.update_at(workspaces, glib::monotonic_time(), animate) {
            start(&self.fixed, &self.state);
        }
    }

    fn update_at(&self, workspaces: &Workspaces, now: i64, animate: bool) -> bool {
        let mut ordered: Vec<_> = workspaces.values().collect();
        ordered.sort_by_key(|workspace| workspace.order);
        let mut items = Vec::new();
        for workspace in ordered {
            items.push(Item::Tag(workspace.tag));
            if workspace.windows.is_empty() {
                items.push(Item::Empty(workspace.tag));
            } else {
                for window in &workspace.windows {
                    items.push(Item::Window(
                        window,
                        window.order == 0 && workspace.order == 0,
                    ));
                }
            }
        }
        let mut state = self.state.borrow_mut();
        let mut keep = BTreeSet::new();
        let mut x = 0.0;
        for item in items {
            if x > 0.0 {
                x += if matches!(item, Item::Tag(_)) {
                    12.0
                } else {
                    4.0
                };
            }
            let key = item.key();
            keep.insert(key.clone());
            let entry = state.entries.entry(key).or_insert_with(|| {
                let tile = item.create();
                tile.widget().set_height_request(HEIGHT);
                self.fixed.put(tile.widget(), x, 0.0);
                Entry {
                    tile,
                    motion: Motion::default(),
                    target: Rect {
                        x,
                        y: 0.0,
                        width: 0.0,
                        height: f64::from(HEIGHT),
                    },
                }
            });
            if let (Item::Window(window, active), Tile::Window(tile)) = (&item, &mut entry.tile) {
                tile.update(window, *active);
            }
            // GtkFixed needs an explicit natural width; discard stale requests before measuring.
            let widget = entry.tile.widget();
            widget.set_width_request(-1);
            let (_, width, _, _) = widget.measure(Orientation::Horizontal, HEIGHT);
            widget.set_width_request(width);
            entry.target = Rect {
                x,
                y: 0.0,
                width: f64::from(width),
                height: f64::from(HEIGHT),
            };
            x += f64::from(width);
        }
        state.entries.retain(|key, entry| {
            if keep.contains(key) {
                true
            } else {
                self.fixed.remove(entry.tile.widget());
                false
            }
        });
        // Draw the focused tile above its neighbors while their paths cross.
        if let Some(button) = state.entries.values().find_map(|entry| match &entry.tile {
            Tile::Window(tile) if tile.button.has_css_class("active") => Some(&tile.button),
            _ => None,
        }) {
            if self.fixed.last_child().as_ref() != Some(button.upcast_ref()) {
                button.insert_before(&self.fixed, None::<&Widget>);
            }
        }
        // Hold the full content width during swaps instead of resizing with each moving tile.
        self.fixed.set_size_request(x.ceil() as i32, HEIGHT);
        advance(&self.fixed, &mut state, now, animate)
    }
}

fn advance(fixed: &Fixed, state: &mut State, now: i64, animate: bool) -> bool {
    let mut moving = false;
    for entry in state.entries.values_mut() {
        moving |= entry.motion.advance(Some(entry.target), now, animate);
        if let Some(position) = entry.motion.current {
            fixed.move_(entry.tile.widget(), position.x, position.y);
        }
    }
    moving
}

fn start(fixed: &Fixed, state: &Rc<RefCell<State>>) {
    if !fixed.is_mapped() || state.borrow().ticking {
        return;
    }
    state.borrow_mut().ticking = true;
    let state = state.clone();
    fixed.add_tick_callback(move |fixed, clock| {
        let mut state = state.borrow_mut();
        let animate = fixed.settings().property::<bool>("gtk-enable-animations");
        if advance(fixed, &mut state, clock.frame_time(), animate) {
            glib::ControlFlow::Continue
        } else {
            state.ticking = false;
            glib::ControlFlow::Break
        }
    });
}

#[cfg(test)]
pub(crate) fn assert_widgets() {
    use crate::workspaces::Workspace;

    let window = |address: &str, tag, order| Window {
        name: format!("Application {address}"),
        info: format!("{address} tooltip"),
        address: address.into(),
        class: "com.mitchellh.ghostty".into(),
        tag,
        order,
    };
    let initial = Workspaces::from([
        (
            1,
            Workspace {
                tag: 1,
                order: 0,
                active: true,
                windows: vec![window("a", 1, 0), window("b", 1, 1)],
            },
        ),
        (
            2,
            Workspace {
                tag: 2,
                order: 1,
                active: false,
                windows: vec![window("c", 2, 2)],
            },
        ),
    ]);
    let tasks = Tasks::new(&initial);
    let panel = Box::builder().css_name("bottom-bar").build();
    let scroller = crate::task_scroller(tasks.widget());
    panel.append(&scroller);
    let width = 160;
    let (_, height, _, _) = panel.measure(Orientation::Vertical, width);
    assert!((38..=40).contains(&height), "task strip is {height}px tall");
    panel.allocate(width, height, -1, None);
    let assert_readable_titles = |tasks: &Tasks| {
        for entry in tasks.state.borrow().entries.values() {
            if let Tile::Window(tile) = &entry.tile {
                assert!(
                    !tile.label.layout().is_ellipsized(),
                    "short title {:?} was clipped: button={}px, label={}px",
                    tile.label.text(),
                    tile.button.width(),
                    tile.label.width()
                );
            }
        }
    };
    assert_readable_titles(&tasks);
    let adjustment = scroller.hadjustment();
    assert!(
        adjustment.upper() > adjustment.page_size(),
        "overflow did not remain scrollable"
    );
    adjustment.set_value(24.0);
    assert_eq!(adjustment.value(), 24.0);
    let key = Key::Window("a".into());
    let original = tasks.state.borrow().entries[&key].tile.widget().clone();
    let old_x = tasks.state.borrow().entries[&key].target.x;
    let switched = Workspaces::from([
        (
            2,
            Workspace {
                tag: 2,
                order: 0,
                active: true,
                windows: vec![window("c", 2, 0)],
            },
        ),
        (
            1,
            Workspace {
                tag: 1,
                order: 1,
                active: false,
                windows: vec![window("b", 1, 1), window("a", 1, 2)],
            },
        ),
    ]);
    assert!(tasks.update_at(&switched, 1, true));
    assert_eq!(
        tasks.widget().last_child().unwrap().widget_name(),
        "c",
        "focused tile must paint above its neighbors"
    );
    {
        let mut state = tasks.state.borrow_mut();
        assert_eq!(
            state.entries[&key].tile.widget(),
            &original,
            "task button was rebuilt"
        );
        let a = &state.entries[&key];
        assert_eq!(
            a.motion.current.unwrap().x,
            old_x,
            "shuffle jumped to its destination"
        );
        let destination = a.target.x;
        assert!(destination > old_x);
        assert!(
            state.entries[&Key::Window("c".into())].target.x
                < state.entries[&Key::Window("b".into())].target.x
        );
        assert!(advance(&tasks.fixed, &mut state, 100_001, true));
        let midway = state.entries[&key].motion.current.unwrap().x;
        assert!(
            midway > old_x && midway < destination,
            "task did not move between frames"
        );
    }
    panel.allocate(width, height, -1, None);
    assert_readable_titles(&tasks);
    assert_eq!(
        adjustment.value(),
        24.0,
        "shuffling reset horizontal scrolling"
    );
    let actual = original.compute_bounds(tasks.widget()).unwrap().x();
    assert!(
        f64::from(actual) > old_x,
        "GTK task allocation did not follow the animation"
    );
    let presented = tasks.state.borrow().entries[&key].motion.current.unwrap().x;
    assert!(tasks.update_at(&initial, 100_002, true));
    assert_eq!(
        tasks.state.borrow().entries[&key].motion.current.unwrap().x,
        presented,
        "rapid retargeting jumped"
    );
    assert!(!advance(
        &tasks.fixed,
        &mut tasks.state.borrow_mut(),
        300_002,
        true
    ));
    assert_eq!(
        tasks.state.borrow().entries[&key].motion.current.unwrap().x,
        old_x
    );
    let mut updated = initial;
    updated.get_mut(&1).unwrap().windows[0].name = "Updated title".into();
    updated.get_mut(&1).unwrap().windows[0].info = "Updated tooltip".into();
    tasks.update_at(&updated, 400_000, false);
    panel.allocate(width, height, -1, None);
    assert_readable_titles(&tasks);
    let state = tasks.state.borrow();
    let Tile::Window(tile) = &state.entries[&key].tile else {
        panic!("window tile expected")
    };
    assert_eq!(tile.label.text(), "Updated title");
    assert_eq!(
        tile.button.tooltip_text().as_deref(),
        Some("Updated tooltip")
    );
    assert!(tile.button.has_css_class("active"));
    assert_eq!(tile.button.upcast_ref::<Widget>(), &original);
    drop(state);
    updated.get_mut(&1).unwrap().windows[0].name = "Long application title ".repeat(20);
    tasks.update_at(&updated, 410_000, false);
    panel.allocate(width, height, -1, None);
    let long_width = original.width();
    {
        let state = tasks.state.borrow();
        let Tile::Window(tile) = &state.entries[&key].tile else {
            panic!("window tile expected")
        };
        assert!(
            tile.label.layout().is_ellipsized(),
            "long titles must stay bounded"
        );
        assert!(
            (100..=240).contains(&tile.label.width()),
            "long title has an incorrect allocation: {}px",
            tile.label.width()
        );
    }
    updated.get_mut(&1).unwrap().windows[0].name = "X".into();
    tasks.update_at(&updated, 420_000, false);
    panel.allocate(width, height, -1, None);
    assert_readable_titles(&tasks);
    assert!(
        original.width() < long_width,
        "a shortened title kept its previous width"
    );
    let mut moved = updated.get_mut(&1).unwrap().windows.remove(0);
    moved.tag = 2;
    updated.get_mut(&2).unwrap().windows.push(moved);
    tasks.update_at(&updated, 500_000, false);
    panel.allocate(width, height, -1, None);
    assert_readable_titles(&tasks);
    assert_eq!(
        tasks.state.borrow().entries[&key].tile.widget(),
        &original,
        "moving a window between workspaces rebuilt its button"
    );
    tasks.update(&Workspaces::new());
    assert!(
        tasks.widget().first_child().is_none(),
        "closed windows were retained"
    );
    assert!(
        !tasks.state.borrow().ticking,
        "unmapped tasks requested animation frames"
    );
}
