use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    time::Duration,
};

use gtk::{
    gdk, gio, glib, prelude::*, Align, Box, Button, Image, Orientation, PopoverMenu, Widget,
};

use crate::hub::Hub;

/* System tray: a StatusNotifierItem host. If no other StatusNotifierWatcher
 * owns the bus name, the bar becomes the watcher itself (items register
 * with it directly); otherwise it registers as a host with the existing one
 * and takes over automatically if that watcher exits. Right-click menus are
 * read from the item's com.canonical.dbusmenu object.
 */

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";
const BUS: &str = "org.freedesktop.DBus";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const ICON_SIZE: i32 = 18;

const WATCHER_XML: &str = r#"<node>
  <interface name="org.kde.StatusNotifierWatcher">
    <method name="RegisterStatusNotifierItem"><arg name="service" type="s" direction="in"/></method>
    <method name="RegisterStatusNotifierHost"><arg name="service" type="s" direction="in"/></method>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
    <signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
    <signal name="StatusNotifierItemUnregistered"><arg type="s"/></signal>
    <signal name="StatusNotifierHostRegistered"/>
    <signal name="StatusNotifierHostUnregistered"/>
  </interface>
</node>"#;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pixmap {
    pub width: i32,
    pub height: i32,
    /* ARGB32 in network byte order, as the specification requires. */
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub bus: String,
    pub path: String,
    pub id: String,
    pub title: String,
    pub status: String,
    pub icon_name: String,
    pub icon_pixmap: Vec<Pixmap>,
    pub attention_icon_name: String,
    pub attention_pixmap: Vec<Pixmap>,
    pub icon_theme_path: String,
    pub tooltip: String,
    pub menu: Option<String>,
    pub item_is_menu: bool,
}

impl Item {
    pub fn key(&self) -> String {
        format!("{}{}", self.bus, self.path)
    }

    pub fn attention(&self) -> bool {
        self.status == "NeedsAttention"
    }

    pub fn visible(&self) -> bool {
        self.status != "Passive"
    }

    pub fn label(&self) -> &str {
        [&self.tooltip, &self.title, &self.id]
            .into_iter()
            .map(|text| text.trim())
            .find(|text| !text.is_empty())
            .unwrap_or("Tray item")
    }

    fn icon(&self) -> (&str, &[Pixmap]) {
        if self.attention()
            && (!self.attention_icon_name.is_empty() || !self.attention_pixmap.is_empty())
        {
            (&self.attention_icon_name, &self.attention_pixmap)
        } else {
            (&self.icon_name, &self.icon_pixmap)
        }
    }
}

/* Split a registered service into (bus name, object path). Items register
 * either a bus name (path defaults to /StatusNotifierItem), an object path
 * on the sender's own connection, or the combined "bus/path" form that
 * watchers report in RegisteredStatusNotifierItems. */
pub fn split_service(service: &str, sender: Option<&str>) -> Option<(String, String)> {
    if service.starts_with('/') {
        return Some((sender?.to_string(), service.to_string()));
    }
    match service.find('/') {
        Some(index) => Some((service[..index].to_string(), service[index..].to_string())),
        None if !service.is_empty() => Some((service.to_string(), ITEM_PATH.to_string())),
        None => None,
    }
}

type Pixmaps = Vec<(i32, i32, Vec<u8>)>;

fn pixmaps(value: Option<&glib::Variant>) -> Vec<Pixmap> {
    value
        .and_then(|value| value.get::<Pixmaps>())
        .unwrap_or_default()
        .into_iter()
        .filter(|(width, height, data)| {
            *width > 0 && *height > 0 && data.len() == (*width * *height * 4) as usize
        })
        .map(|(width, height, data)| Pixmap {
            width,
            height,
            data,
        })
        .collect()
}

fn parse_item(bus: &str, path: &str, properties: &glib::VariantDict) -> Item {
    let text = |name: &str| {
        properties
            .lookup_value(name, None)
            .and_then(|value| value.str().map(str::to_string))
            .unwrap_or_default()
    };
    let tooltip = properties
        .lookup_value("ToolTip", None)
        .and_then(|value| value.get::<(String, Pixmaps, String, String)>())
        .map(|(_, _, title, description)| {
            [title, description]
                .into_iter()
                .filter(|text| !text.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let menu = text("Menu");
    Item {
        bus: bus.into(),
        path: path.into(),
        id: text("Id"),
        title: text("Title"),
        status: text("Status"),
        icon_name: text("IconName"),
        icon_pixmap: pixmaps(properties.lookup_value("IconPixmap", None).as_ref()),
        attention_icon_name: text("AttentionIconName"),
        attention_pixmap: pixmaps(
            properties
                .lookup_value("AttentionIconPixmap", None)
                .as_ref(),
        ),
        icon_theme_path: text("IconThemePath"),
        tooltip,
        menu: (!menu.is_empty() && menu != "/").then_some(menu),
        item_is_menu: properties
            .lookup_value("ItemIsMenu", None)
            .and_then(|value| value.get::<bool>())
            .unwrap_or(false),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Starting,
    Watcher,
    Host,
}

pub struct Tray {
    connection: RefCell<Option<gio::DBusConnection>>,
    mode: Cell<Mode>,
    items: RefCell<BTreeMap<String, Item>>,
    signals: RefCell<BTreeMap<String, gio::SignalSubscription>>,
    watcher_signals: RefCell<Vec<gio::SignalSubscription>>,
    pending: RefCell<BTreeSet<String>>,
    hub: Rc<Hub<Vec<Item>>>,
    _owners: RefCell<Vec<gio::OwnerId>>,
    _registration: RefCell<Option<gio::RegistrationId>>,
    _name_owners: RefCell<Option<gio::SignalSubscription>>,
}

impl Tray {
    pub fn start() -> Rc<Self> {
        let tray = Rc::new(Self {
            connection: RefCell::default(),
            mode: Cell::new(Mode::Starting),
            items: RefCell::default(),
            signals: RefCell::default(),
            watcher_signals: RefCell::default(),
            pending: RefCell::default(),
            hub: Hub::new(),
            _owners: RefCell::default(),
            _registration: RefCell::default(),
            _name_owners: RefCell::default(),
        });
        tray.hub.publish(Vec::new());
        let starting = tray.clone();
        glib::spawn_future_local(async move {
            if let Err(error) = starting.connect().await {
                eprintln!("epic-bar-rs: tray unavailable: {error}");
            }
        });
        tray
    }

    pub fn hub(&self) -> &Rc<Hub<Vec<Item>>> {
        &self.hub
    }

    async fn connect(self: &Rc<Self>) -> Result<(), String> {
        let connection = gio::bus_get_future(gio::BusType::Session)
            .await
            .map_err(|error| error.to_string())?;
        *self.connection.borrow_mut() = Some(connection.clone());

        let info = gio::DBusNodeInfo::for_xml(WATCHER_XML).map_err(|error| error.to_string())?;
        let interface = info
            .lookup_interface(WATCHER)
            .ok_or("missing watcher interface")?;
        let calls = Rc::downgrade(self);
        let properties = Rc::downgrade(self);
        let registration = connection
            .register_object(WATCHER_PATH, &interface)
            .method_call(move |_, sender, _, _, method, parameters, invocation| {
                let Some(tray) = calls.upgrade() else {
                    invocation
                        .return_dbus_error("org.freedesktop.DBus.Error.Failed", "tray stopped");
                    return;
                };
                let service = parameters.get::<(String,)>().map(|(service,)| service);
                match (method, service) {
                    ("RegisterStatusNotifierItem", Some(service)) => {
                        match split_service(&service, sender) {
                            Some((bus, path)) => {
                                invocation.return_value(None);
                                tray.add(bus, path);
                            }
                            None => invocation.return_dbus_error(
                                "org.freedesktop.DBus.Error.InvalidArgs",
                                "invalid status notifier item",
                            ),
                        }
                    }
                    ("RegisterStatusNotifierHost", Some(_)) => {
                        invocation.return_value(None);
                        tray.emit("StatusNotifierHostRegistered", None);
                    }
                    _ => invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.UnknownMethod",
                        "unknown method",
                    ),
                }
            })
            .property(move |_, _, _, _, name| {
                let Some(tray) = properties.upgrade() else {
                    return false.to_variant();
                };
                match name {
                    "RegisteredStatusNotifierItems" => tray
                        .items
                        .borrow()
                        .values()
                        .map(|item| format!("{}{}", item.bus, item.path))
                        .collect::<Vec<_>>()
                        .to_variant(),
                    "IsStatusNotifierHostRegistered" => true.to_variant(),
                    "ProtocolVersion" => 0i32.to_variant(),
                    _ => false.to_variant(),
                }
            })
            .build()
            .map_err(|error| format!("cannot export the tray watcher: {error}"))?;
        *self._registration.borrow_mut() = Some(registration);

        let owners = Rc::downgrade(self);
        let name_owners = connection.subscribe_to_signal(
            Some(BUS),
            Some(BUS),
            Some("NameOwnerChanged"),
            Some("/org/freedesktop/DBus"),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                if let (Some(tray), Some((name, _, new))) = (
                    owners.upgrade(),
                    signal.parameters.get::<(String, String, String)>(),
                ) {
                    if new.is_empty() {
                        tray.vanished(&name);
                    }
                }
            },
        );
        *self._name_owners.borrow_mut() = Some(name_owners);

        let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
        let host_owner = gio::bus_own_name_on_connection(
            &connection,
            &host,
            gio::BusNameOwnerFlags::NONE,
            |_, _| {},
            |_, _| {},
        );
        // Queue for the watcher name: if another watcher holds it we act as
        // a host and take over when it exits.
        let acquired = Rc::downgrade(self);
        let lost = Rc::downgrade(self);
        let watcher_owner = gio::bus_own_name_on_connection(
            &connection,
            WATCHER,
            gio::BusNameOwnerFlags::NONE,
            move |_, _| {
                if let Some(tray) = acquired.upgrade() {
                    tray.become_watcher();
                }
            },
            move |_, _| {
                if let Some(tray) = lost.upgrade() {
                    tray.become_host();
                }
            },
        );
        self._owners
            .borrow_mut()
            .extend([host_owner, watcher_owner]);
        Ok(())
    }

    fn connection(&self) -> Option<gio::DBusConnection> {
        self.connection.borrow().clone()
    }

    fn emit(&self, signal: &str, parameters: Option<glib::Variant>) {
        if self.mode.get() != Mode::Watcher {
            return;
        }
        if let Some(connection) = self.connection() {
            if let Err(error) =
                connection.emit_signal(None, WATCHER_PATH, WATCHER, signal, parameters.as_ref())
            {
                eprintln!("epic-bar-rs: cannot emit {signal}: {error}");
            }
        }
    }

    fn clear(&self) {
        self.items.borrow_mut().clear();
        self.signals.borrow_mut().clear();
        self.watcher_signals.borrow_mut().clear();
        self.publish();
    }

    fn become_watcher(self: &Rc<Self>) {
        if self.mode.replace(Mode::Watcher) == Mode::Watcher {
            return;
        }
        // Items re-register when they see the new watcher appear.
        self.clear();
        self.emit("StatusNotifierHostRegistered", None);
    }

    fn become_host(self: &Rc<Self>) {
        if self.mode.replace(Mode::Host) == Mode::Host {
            return;
        }
        self.clear();
        let Some(connection) = self.connection() else {
            return;
        };
        for (signal, added) in [
            ("StatusNotifierItemRegistered", true),
            ("StatusNotifierItemUnregistered", false),
        ] {
            let weak = Rc::downgrade(self);
            let subscription = connection.subscribe_to_signal(
                Some(WATCHER),
                Some(WATCHER),
                Some(signal),
                Some(WATCHER_PATH),
                None,
                gio::DBusSignalFlags::NONE,
                move |event| {
                    let (Some(tray), Some((service,))) =
                        (weak.upgrade(), event.parameters.get::<(String,)>())
                    else {
                        return;
                    };
                    let Some((bus, path)) = split_service(&service, None) else {
                        return;
                    };
                    if added {
                        tray.add(bus, path);
                    } else {
                        tray.remove(&format!("{bus}{path}"));
                    }
                },
            );
            self.watcher_signals.borrow_mut().push(subscription);
        }
        let tray = self.clone();
        glib::spawn_future_local(async move {
            let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
            if let Err(error) = connection
                .call_future(
                    Some(WATCHER),
                    WATCHER_PATH,
                    WATCHER,
                    "RegisterStatusNotifierHost",
                    Some(&(host,).to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    2000,
                )
                .await
            {
                eprintln!("epic-bar-rs: cannot register as a tray host: {error}");
            }
            let reply = connection
                .call_future(
                    Some(WATCHER),
                    WATCHER_PATH,
                    PROPERTIES,
                    "Get",
                    Some(&(WATCHER, "RegisteredStatusNotifierItems").to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    2000,
                )
                .await;
            let services = reply
                .ok()
                .and_then(|reply| reply.get::<(glib::Variant,)>())
                .and_then(|(value,)| value.get::<Vec<String>>())
                .unwrap_or_default();
            for service in services {
                if let Some((bus, path)) = split_service(&service, None) {
                    tray.add(bus, path);
                }
            }
        });
    }

    fn add(self: &Rc<Self>, bus: String, path: String) {
        let key = format!("{bus}{path}");
        if !self.signals.borrow().contains_key(&key) {
            if let Some(connection) = self.connection() {
                let weak = Rc::downgrade(self);
                let refresh_key = key.clone();
                let subscription = connection.subscribe_to_signal(
                    Some(&bus),
                    Some(ITEM),
                    None,
                    Some(&path),
                    None,
                    gio::DBusSignalFlags::NONE,
                    move |_| {
                        if let Some(tray) = weak.upgrade() {
                            tray.schedule(refresh_key.clone());
                        }
                    },
                );
                self.signals.borrow_mut().insert(key.clone(), subscription);
            }
            self.emit(
                "StatusNotifierItemRegistered",
                Some((key.clone(),).to_variant()),
            );
        }
        self.items.borrow_mut().entry(key.clone()).or_insert(Item {
            bus,
            path,
            status: "Passive".into(),
            ..Item::default()
        });
        self.schedule(key);
    }

    fn remove(&self, key: &str) {
        let removed = self.items.borrow_mut().remove(key).is_some();
        self.signals.borrow_mut().remove(key);
        if removed {
            self.emit("StatusNotifierItemUnregistered", Some((key,).to_variant()));
            self.publish();
        }
    }

    fn vanished(&self, name: &str) {
        let keys: Vec<_> = self
            .items
            .borrow()
            .values()
            .filter(|item| item.bus == name)
            .map(Item::key)
            .collect();
        for key in keys {
            self.remove(&key);
        }
    }

    /* Coalesce bursts of New* signals into one property read. */
    fn schedule(self: &Rc<Self>, key: String) {
        if !self.pending.borrow_mut().insert(key.clone()) {
            return;
        }
        let tray = self.clone();
        glib::spawn_future_local(async move {
            glib::timeout_future(Duration::from_millis(80)).await;
            tray.pending.borrow_mut().remove(&key);
            tray.refresh(&key).await;
        });
    }

    async fn refresh(&self, key: &str) {
        let Some((bus, path)) = self
            .items
            .borrow()
            .get(key)
            .map(|item| (item.bus.clone(), item.path.clone()))
        else {
            return;
        };
        let Some(connection) = self.connection() else {
            return;
        };
        let reply = connection
            .call_future(
                Some(&bus),
                &path,
                PROPERTIES,
                "GetAll",
                Some(&(ITEM,).to_variant()),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                3000,
            )
            .await;
        match reply {
            Ok(reply) => {
                let properties = glib::VariantDict::new(Some(&reply.child_value(0)));
                let item = parse_item(&bus, &path, &properties);
                if let Some(existing) = self.items.borrow_mut().get_mut(key) {
                    *existing = item;
                }
                self.publish();
            }
            Err(error) => {
                eprintln!("epic-bar-rs: cannot read tray item {key}: {error}");
                self.remove(key);
            }
        }
    }

    fn publish(&self) {
        let items: Vec<_> = self.items.borrow().values().cloned().collect();
        self.hub.publish(items);
    }

    fn call(&self, item: &Item, method: &str, parameters: glib::Variant) {
        let Some(connection) = self.connection() else {
            return;
        };
        let (bus, path, method) = (item.bus.clone(), item.path.clone(), method.to_string());
        glib::spawn_future_local(async move {
            if let Err(error) = connection
                .call_future(
                    Some(&bus),
                    &path,
                    ITEM,
                    &method,
                    Some(&parameters),
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    3000,
                )
                .await
            {
                eprintln!("epic-bar-rs: tray {method} on {bus}{path} failed: {error}");
            }
        });
    }

    fn show_menu(&self, item: &Item, anchor: &Widget) {
        let (Some(connection), Some(menu)) = (self.connection(), item.menu.clone()) else {
            self.call(item, "ContextMenu", (0i32, 0i32).to_variant());
            return;
        };
        let bus = item.bus.clone();
        let anchor = anchor.downgrade();
        glib::spawn_future_local(async move {
            // AboutToShow lets the application rebuild the menu first; many
            // implementations do not support it, so failures are ignored.
            let _ = connection
                .call_future(
                    Some(&bus),
                    &menu,
                    MENU,
                    "AboutToShow",
                    Some(&(0i32,).to_variant()),
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    1000,
                )
                .await;
            let layout = connection
                .call_future(
                    Some(&bus),
                    &menu,
                    MENU,
                    "GetLayout",
                    Some(&(0i32, -1i32, Vec::<String>::new()).to_variant()),
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    3000,
                )
                .await;
            let layout = match layout {
                Ok(layout) => layout.child_value(1),
                Err(error) => {
                    eprintln!("epic-bar-rs: cannot read tray menu of {bus}: {error}");
                    return;
                }
            };
            let Some(anchor) = anchor.upgrade() else {
                return;
            };
            let actions = gio::SimpleActionGroup::new();
            let model = gio::Menu::new();
            build_menu(&layout, &model, &actions, &connection, &bus, &menu);
            let popover = PopoverMenu::from_model(Some(&model));
            popover.add_css_class("bar-popover");
            popover.add_css_class("tray-menu");
            popover.insert_action_group("dbusmenu", Some(&actions));
            popover.set_parent(&anchor);
            popover.connect_closed(|popover| {
                let popover = popover.clone();
                // Unparent after the activated action (if any) has run.
                glib::idle_add_local_once(move || popover.unparent());
            });
            popover.popup();
        });
    }
}

fn menu_property(properties: &glib::VariantDict, name: &str) -> Option<glib::Variant> {
    properties.lookup_value(name, None)
}

/* Translate one dbusmenu layout node's children into `model`. Separators
 * start a new section, matching how GTK draws menu separators. */
fn build_menu(
    node: &glib::Variant,
    model: &gio::Menu,
    actions: &gio::SimpleActionGroup,
    connection: &gio::DBusConnection,
    bus: &str,
    menu_path: &str,
) {
    let children = node.child_value(2);
    let mut section = gio::Menu::new();
    for index in 0..children.n_children() {
        let child = children.child_value(index);
        let child = child.as_variant().unwrap_or(child);
        let Some(id) = child.child_value(0).get::<i32>() else {
            continue;
        };
        let properties = glib::VariantDict::new(Some(&child.child_value(1)));
        let flag = |name: &str, default: bool| {
            menu_property(&properties, name)
                .and_then(|value| value.get::<bool>())
                .unwrap_or(default)
        };
        let text = |name: &str| {
            menu_property(&properties, name)
                .and_then(|value| value.str().map(str::to_string))
                .unwrap_or_default()
        };
        if !flag("visible", true) {
            continue;
        }
        if text("type") == "separator" {
            if section.n_items() > 0 {
                model.append_section(None, &section);
                section = gio::Menu::new();
            }
            continue;
        }
        let label = text("label");
        if text("children-display") == "submenu" {
            let submenu = gio::Menu::new();
            build_menu(&child, &submenu, actions, connection, bus, menu_path);
            section.append_submenu(Some(&label), &submenu);
            continue;
        }
        let name = format!("item{}", id.unsigned_abs());
        let toggled = menu_property(&properties, "toggle-state")
            .and_then(|value| value.get::<i32>())
            .map(|state| state == 1);
        let action = match text("toggle-type").as_str() {
            "checkmark" | "radio" => {
                gio::SimpleAction::new_stateful(&name, None, &toggled.unwrap_or(false).to_variant())
            }
            _ => gio::SimpleAction::new(&name, None),
        };
        action.set_enabled(flag("enabled", true));
        let (connection, bus, menu_path) =
            (connection.clone(), bus.to_string(), menu_path.to_string());
        action.connect_activate(move |_, _| {
            let (connection, bus, menu_path) = (connection.clone(), bus.clone(), menu_path.clone());
            glib::spawn_future_local(async move {
                let event = (id, "clicked", 0i32.to_variant(), 0u32).to_variant();
                if let Err(error) = connection
                    .call_future(
                        Some(&bus),
                        &menu_path,
                        MENU,
                        "Event",
                        Some(&event),
                        None,
                        gio::DBusCallFlags::NO_AUTO_START,
                        3000,
                    )
                    .await
                {
                    eprintln!("epic-bar-rs: tray menu event on {bus} failed: {error}");
                }
            });
        });
        actions.add_action(&action);
        section.append(Some(&label), Some(&format!("dbusmenu.{name}")));
    }
    if section.n_items() > 0 {
        model.append_section(None, &section);
    }
}

fn texture(pixmaps: &[Pixmap], size: i32) -> Option<gdk::MemoryTexture> {
    // Smallest pixmap at least as large as needed, else the largest one.
    let pixmap = pixmaps
        .iter()
        .filter(|pixmap| pixmap.width >= size)
        .min_by_key(|pixmap| pixmap.width)
        .or_else(|| pixmaps.iter().max_by_key(|pixmap| pixmap.width))?;
    Some(gdk::MemoryTexture::new(
        pixmap.width,
        pixmap.height,
        gdk::MemoryFormat::A8r8g8b8,
        &glib::Bytes::from(&pixmap.data),
        pixmap.width as usize * 4,
    ))
}

fn icon(item: &Item, theme: &gtk::IconTheme, scale: i32) -> Image {
    let (name, pixmaps) = item.icon();
    let image = if name.starts_with('/') {
        Image::from_file(name)
    } else if !name.is_empty() && theme.has_icon(name) {
        Image::from_icon_name(name)
    } else if let Some(texture) = texture(pixmaps, ICON_SIZE * scale) {
        Image::from_paintable(Some(&texture))
    } else if !name.is_empty() {
        // Let GTK's own lookup (and its fallbacks) try once more.
        Image::from_icon_name(name)
    } else {
        Image::from_icon_name("application-x-executable-symbolic")
    };
    image.set_pixel_size(ICON_SIZE);
    image
}

pub fn widget(tray: &Rc<Tray>) -> Box {
    let container = Box::builder()
        .orientation(Orientation::Horizontal)
        .css_name("tray")
        .valign(Align::Center)
        .spacing(2)
        .visible(false)
        .build();
    let weak = container.downgrade();
    let tray_weak = Rc::downgrade(tray);
    let search_paths = Rc::new(RefCell::new(BTreeSet::<String>::new()));
    let shown = Rc::new(RefCell::new(Vec::<Item>::new()));
    tray.hub().subscribe(move |items| {
        let Some(container) = weak.upgrade() else {
            return false;
        };
        let items: Vec<_> = items
            .iter()
            .filter(|item| item.visible())
            .cloned()
            .collect();
        if *shown.borrow() == items {
            return true;
        }
        let theme = gtk::IconTheme::for_display(&container.display());
        for item in &items {
            if !item.icon_theme_path.is_empty()
                && search_paths
                    .borrow_mut()
                    .insert(item.icon_theme_path.clone())
            {
                theme.add_search_path(&item.icon_theme_path);
            }
        }
        while let Some(child) = container.first_child() {
            container.remove(&child);
        }
        for item in &items {
            let button = Button::builder()
                .css_classes(["bar-button", "tray-item"])
                .tooltip_text(item.label())
                .focus_on_click(false)
                .child(&icon(item, &theme, container.scale_factor()))
                .build();
            if item.attention() {
                button.add_css_class("urgent");
            }
            let click = gtk::GestureClick::builder().button(0).build();
            let (tray, clicked_item) = (tray_weak.clone(), item.clone());
            click.connect_released(move |gesture, _, _, _| {
                let Some(tray) = tray.upgrade() else { return };
                let Some(widget) = gesture.widget() else {
                    return;
                };
                gesture.set_state(gtk::EventSequenceState::Claimed);
                match gesture.current_button() {
                    gdk::BUTTON_PRIMARY if clicked_item.item_is_menu => {
                        tray.show_menu(&clicked_item, &widget)
                    }
                    gdk::BUTTON_PRIMARY => {
                        tray.call(&clicked_item, "Activate", (0i32, 0i32).to_variant())
                    }
                    gdk::BUTTON_MIDDLE => tray.call(
                        &clicked_item,
                        "SecondaryActivate",
                        (0i32, 0i32).to_variant(),
                    ),
                    gdk::BUTTON_SECONDARY => tray.show_menu(&clicked_item, &widget),
                    _ => {}
                }
            });
            button.add_controller(click);
            let scroll = gtk::EventControllerScroll::new(
                gtk::EventControllerScrollFlags::VERTICAL
                    | gtk::EventControllerScrollFlags::DISCRETE,
            );
            let (tray, scrolled_item) = (tray_weak.clone(), item.clone());
            scroll.connect_scroll(move |_, _, dy| {
                if let Some(tray) = tray.upgrade() {
                    let delta = if dy < 0.0 { -1i32 } else { 1i32 };
                    tray.call(&scrolled_item, "Scroll", (delta, "vertical").to_variant());
                }
                glib::Propagation::Stop
            });
            button.add_controller(scroll);
            container.append(&button);
        }
        container.set_visible(!items.is_empty());
        *shown.borrow_mut() = items;
        true
    });
    container
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_resolve_to_bus_and_path() {
        assert_eq!(
            split_service("org.kde.StatusNotifierItem-1-1", None),
            Some(("org.kde.StatusNotifierItem-1-1".into(), ITEM_PATH.into()))
        );
        assert_eq!(
            split_service("/org/ayatana/NotificationItem/steam", Some(":1.42")),
            Some((":1.42".into(), "/org/ayatana/NotificationItem/steam".into()))
        );
        assert_eq!(split_service("/only/path", None), None);
        assert_eq!(
            split_service(":1.9/StatusNotifierItem", None),
            Some((":1.9".into(), "/StatusNotifierItem".into()))
        );
        assert_eq!(split_service("", None), None);
    }

    #[test]
    fn item_properties_parse_tooltips_menus_and_valid_pixmaps() {
        let properties = glib::VariantDict::new(None);
        properties.insert_value("Id", &"steam".to_variant());
        properties.insert_value("Status", &"NeedsAttention".to_variant());
        properties.insert_value("IconName", &"steam".to_variant());
        properties.insert_value("AttentionIconName", &"steam-alert".to_variant());
        properties.insert_value(
            "ToolTip",
            &(
                "",
                Vec::<(i32, i32, Vec<u8>)>::new(),
                "Steam",
                "Friends online",
            )
                .to_variant(),
        );
        properties.insert_value(
            "IconPixmap",
            &vec![(1i32, 1i32, vec![255u8, 1, 2, 3]), (2, 2, vec![0u8; 3])].to_variant(),
        );
        properties.insert_value(
            "Menu",
            &glib::variant::ObjectPath::try_from("/MenuBar".to_string())
                .unwrap()
                .to_variant(),
        );
        let item = parse_item(":1.5", ITEM_PATH, &properties);
        assert_eq!(item.label(), "Steam\nFriends online");
        assert_eq!(item.menu.as_deref(), Some("/MenuBar"));
        assert_eq!(item.icon_pixmap.len(), 1, "malformed pixmaps are dropped");
        assert!(item.attention() && item.visible());
        assert_eq!(item.icon().0, "steam-alert");
        assert_eq!(item.key(), ":1.5/StatusNotifierItem");
        let empty = parse_item(":1.6", "/x", &glib::VariantDict::new(None));
        assert_eq!(empty.label(), "Tray item");
        assert_eq!(empty.menu, None);
    }
}
