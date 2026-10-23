use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gtk::{gio, glib, prelude::*, Box, Button, Label, Orientation};

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const BUS: &str = "org.freedesktop.DBus";
const PREFIX: &str = "org.mpris.MediaPlayer2.";
type Properties = HashMap<String, glib::Variant>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Playback {
    Playing,
    Paused,
    Stopped,
}

#[derive(Clone, Debug)]
struct Player {
    name: String,
    playback: Playback,
    title: String,
    artist: String,
    can_control: bool,
    can_play: bool,
    can_pause: bool,
    can_previous: bool,
    can_next: bool,
}

fn property<T: glib::variant::FromVariant>(
    properties: &Properties,
    name: &str,
) -> Result<T, String> {
    properties
        .get(name)
        .and_then(glib::Variant::get)
        .ok_or_else(|| format!("missing or invalid MPRIS property {name}"))
}

fn optional_text(metadata: &Properties, name: &str) -> Result<String, String> {
    match metadata.get(name) {
        Some(value) => value
            .get()
            .ok_or_else(|| format!("invalid MPRIS metadata {name}")),
        None => Ok(String::new()),
    }
}

fn parse_player(name: &str, properties: &Properties) -> Result<Player, String> {
    let playback: String = property(properties, "PlaybackStatus")?;
    let playback = match playback.as_str() {
        "Playing" => Playback::Playing,
        "Paused" => Playback::Paused,
        "Stopped" => Playback::Stopped,
        _ => return Err(format!("invalid MPRIS playback state: {playback}")),
    };
    let metadata: Properties = property(properties, "Metadata")?;
    let artist = match metadata.get("xesam:artist") {
        Some(value) => value
            .get::<Vec<String>>()
            .ok_or("invalid MPRIS artist metadata")?
            .join(", "),
        None => String::new(),
    };
    Ok(Player {
        name: name.into(),
        playback,
        title: optional_text(&metadata, "xesam:title")?,
        artist,
        can_control: property(properties, "CanControl")?,
        can_play: property(properties, "CanPlay")?,
        can_pause: property(properties, "CanPause")?,
        can_previous: property(properties, "CanGoPrevious")?,
        can_next: property(properties, "CanGoNext")?,
    })
}

fn select(players: Vec<Player>, current: Option<&str>) -> Option<Player> {
    players
        .into_iter()
        .filter(|player| player.playback != Playback::Stopped)
        .min_by_key(|player| {
            (
                player.playback != Playback::Playing,
                Some(player.name.as_str()) != current,
                player.name.clone(),
            )
        })
}

async fn scan(connection: &gio::DBusConnection) -> Result<Vec<Player>, String> {
    let reply = connection
        .call_future(
            Some(BUS),
            "/org/freedesktop/DBus",
            BUS,
            "ListNames",
            None,
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            2000,
        )
        .await
        .map_err(|error| format!("cannot list media players: {error}"))?;
    let (names,) = reply
        .get::<(Vec<String>,)>()
        .ok_or("invalid D-Bus ListNames reply")?;
    let mut players = Vec::new();
    let mut last_error = None;
    for name in names.into_iter().filter(|name| name.starts_with(PREFIX)) {
        let result = async {
            let reply = connection
                .call_future(
                    Some(&name),
                    PATH,
                    "org.freedesktop.DBus.Properties",
                    "GetAll",
                    Some(&(PLAYER,).to_variant()),
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    2000,
                )
                .await
                .map_err(|error| error.to_string())?;
            let (properties,) = reply
                .get::<(Properties,)>()
                .ok_or("invalid MPRIS GetAll reply")?;
            parse_player(&name, &properties)
        }
        .await;
        match result {
            Ok(player) => players.push(player),
            Err(error) => {
                let message = format!("cannot read media player {name}: {error}");
                eprintln!("epic-bar-rs: {message}");
                last_error = Some(message);
            }
        }
    }
    if players.is_empty() {
        if let Some(error) = last_error {
            return Err(error);
        }
    }
    Ok(players)
}

async fn control(connection: &gio::DBusConnection, name: &str, method: &str) -> Result<(), String> {
    connection
        .call_future(
            Some(name),
            PATH,
            PLAYER,
            method,
            None,
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            2000,
        )
        .await
        .map(|_| ())
        .map_err(|error| format!("cannot send {method} to {name}: {error}"))
}

fn subscribe(
    connection: &gio::DBusConnection,
    tx: &async_channel::Sender<()>,
) -> Vec<gio::SignalSubscription> {
    let changed = tx.clone();
    let properties = connection.subscribe_to_signal(
        None,
        Some("org.freedesktop.DBus.Properties"),
        Some("PropertiesChanged"),
        Some(PATH),
        Some(PLAYER),
        gio::DBusSignalFlags::NONE,
        move |_| {
            let _ = changed.try_send(());
        },
    );
    let changed = tx.clone();
    let owners = connection.subscribe_to_signal(
        Some(BUS),
        Some(BUS),
        Some("NameOwnerChanged"),
        Some("/org/freedesktop/DBus"),
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            if let Some((name, _, _)) = signal.parameters.get::<(String, String, String)>() {
                if name.starts_with(PREFIX) {
                    let _ = changed.try_send(());
                }
            }
        },
    );
    vec![properties, owners]
}

#[derive(Clone)]
pub struct Media {
    pub widget: Box,
    title: crate::marquee::Marquee,
    previous: Button,
    play: Button,
    next: Button,
    selected: Rc<RefCell<Option<Player>>>,
    connection: Rc<RefCell<Option<gio::DBusConnection>>>,
    refresh: async_channel::Sender<()>,
    events: async_channel::Receiver<()>,
}

impl Media {
    pub fn new() -> Self {
        let widget = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(2)
            .css_classes(["media-widget"])
            .visible(false)
            .build();
        let title = Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(28)
            .single_line_mode(true)
            .css_classes(["media-title"])
            .build();
        let title = crate::marquee::Marquee::new(title, crate::marquee::Mode::Continuous);
        let button = |icon: &str, tooltip: &str| {
            Button::builder()
                .icon_name(icon)
                .tooltip_text(tooltip)
                .css_classes(["bar-button", "media-control"])
                .focus_on_click(false)
                .build()
        };
        let previous = button("media-skip-backward-symbolic", "Previous track");
        let play = button("media-playback-start-symbolic", "Play");
        let next = button("media-skip-forward-symbolic", "Next track");
        widget.append(&previous);
        widget.append(&play);
        widget.append(&next);
        widget.append(title.widget());
        let (refresh, events) = async_channel::bounded(1);
        let media = Self {
            widget,
            title,
            previous,
            play,
            next,
            selected: Rc::new(RefCell::new(None)),
            connection: Rc::new(RefCell::new(None)),
            refresh,
            events,
        };
        for (button, method) in [
            (&media.previous, "Previous"),
            (&media.play, "PlayPause"),
            (&media.next, "Next"),
        ] {
            let selected = media.selected.clone();
            let connection = media.connection.clone();
            let refresh = media.refresh.clone();
            let weak_widget = media.widget.downgrade();
            button.connect_clicked(move |_| {
                let Some(player) = selected.borrow().clone() else {
                    return;
                };
                let Some(connection) = connection.borrow().clone() else {
                    return;
                };
                let (refresh, weak_widget) = (refresh.clone(), weak_widget.clone());
                glib::spawn_future_local(async move {
                    if let Err(error) = control(&connection, &player.name, method).await {
                        eprintln!("epic-bar-rs: {error}");
                        if let Some(widget) = weak_widget.upgrade() {
                            widget.set_tooltip_text(Some(&error));
                        }
                    }
                    let _ = refresh.try_send(());
                });
            });
        }
        media
    }

    fn render(&self, player: Option<Player>) {
        self.widget.remove_css_class("warning");
        self.widget.set_visible(player.is_some());
        if let Some(player) = &player {
            let title = if player.title.is_empty() {
                player
                    .name
                    .strip_prefix(PREFIX)
                    .unwrap_or(&player.name)
                    .to_string()
            } else if player.artist.is_empty() {
                player.title.clone()
            } else {
                format!("{} - {}", player.artist, player.title)
            };
            self.title.set_text(&title);
            self.widget
                .set_tooltip_text(Some(&format!("{title}\n{}", player.name)));
            self.previous
                .set_sensitive(player.can_control && player.can_previous);
            self.next
                .set_sensitive(player.can_control && player.can_next);
            let playing = player.playback == Playback::Playing;
            self.play.set_icon_name(if playing {
                "media-playback-pause-symbolic"
            } else {
                "media-playback-start-symbolic"
            });
            self.play
                .set_tooltip_text(Some(if playing { "Pause" } else { "Play" }));
            self.play.set_sensitive(
                player.can_control
                    && if playing {
                        player.can_pause
                    } else {
                        player.can_play
                    },
            );
        }
        *self.selected.borrow_mut() = player;
    }

    fn error(&self, message: &str) {
        eprintln!("epic-bar-rs: media monitor: {message}");
        self.render(None);
        self.widget.add_css_class("warning");
        self.widget.set_visible(true);
        self.title.set_text("Media unavailable");
        self.widget.set_tooltip_text(Some(message));
        for button in [&self.previous, &self.play, &self.next] {
            button.set_sensitive(false);
        }
    }

    pub fn start(&self) {
        let media = self.clone();
        glib::spawn_future_local(async move {
            loop {
                let connection = match gio::bus_get_future(gio::BusType::Session).await {
                    Ok(connection) => connection,
                    Err(error) => {
                        media.error(&error.to_string());
                        glib::timeout_future_seconds(2).await;
                        continue;
                    }
                };
                connection.set_exit_on_close(false);
                *media.connection.borrow_mut() = Some(connection.clone());
                let subscriptions = subscribe(&connection, &media.refresh);
                let refresh = media.refresh.clone();
                let closed = connection.connect_closed(move |_, _, _| {
                    let _ = refresh.try_send(());
                });
                let _ = media.refresh.try_send(());
                while media.events.recv().await.is_ok() {
                    if connection.is_closed() {
                        break;
                    }
                    match scan(&connection).await {
                        Ok(players) => {
                            let current = media
                                .selected
                                .borrow()
                                .as_ref()
                                .map(|player| player.name.clone());
                            media.render(select(players, current.as_deref()));
                        }
                        Err(error) => {
                            media.error(&error);
                            glib::timeout_future_seconds(2).await;
                            let _ = media.refresh.try_send(());
                        }
                    }
                }
                drop(subscriptions);
                connection.disconnect(closed);
                *media.connection.borrow_mut() = None;
                media.error("Session bus disconnected; reconnecting");
                glib::timeout_future_seconds(2).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn properties(playback: &str) -> Properties {
        HashMap::from([
            ("PlaybackStatus".into(), playback.to_variant()),
            (
                "Metadata".into(),
                HashMap::from([
                    ("xesam:title".to_string(), "Track <one> & two".to_variant()),
                    ("xesam:artist".to_string(), vec!["Artist"].to_variant()),
                ])
                .to_variant(),
            ),
            ("CanControl".into(), true.to_variant()),
            ("CanPlay".into(), true.to_variant()),
            ("CanPause".into(), true.to_variant()),
            ("CanGoPrevious".into(), false.to_variant()),
            ("CanGoNext".into(), true.to_variant()),
        ])
    }

    #[test]
    fn media_selection_keeps_paused_players_but_prefers_playing() {
        let paused = parse_player("org.mpris.MediaPlayer2.a", &properties("Paused")).unwrap();
        let playing = parse_player("org.mpris.MediaPlayer2.b", &properties("Playing")).unwrap();
        assert_eq!(
            select(vec![paused.clone()], None).unwrap().playback,
            Playback::Paused
        );
        assert_eq!(
            select(vec![paused.clone(), playing.clone()], Some(&paused.name))
                .unwrap()
                .name,
            playing.name
        );
        let stopped = parse_player("org.mpris.MediaPlayer2.c", &properties("Stopped")).unwrap();
        assert!(select(vec![stopped], None).is_none());
        assert!(parse_player("bad", &properties("unknown")).is_err());
        let mut broken = properties("Playing");
        broken.insert("CanControl".into(), "wrong type".to_variant());
        assert!(parse_player("bad", &broken).is_err());
    }

    #[test]
    #[ignore = "requires an isolated session bus (dbus-run-session)"]
    fn mpris_lifecycle_and_controls() {
        let context = glib::MainContext::new();
        context.block_on(async {
            let connection = gio::bus_get_future(gio::BusType::Session).await.unwrap();
            let name = "org.mpris.MediaPlayer2.EpicBarFixture";
            let data = Rc::new(RefCell::new(properties("Playing")));
            let calls = Rc::new(RefCell::new(Vec::<String>::new()));
            let xml = r#"<node><interface name="org.mpris.MediaPlayer2.Player">
                <method name="PlayPause"/><method name="Next"/><method name="Previous"/>
                <property name="PlaybackStatus" type="s" access="read"/>
                <property name="Metadata" type="a{sv}" access="read"/>
                <property name="CanControl" type="b" access="read"/>
                <property name="CanPlay" type="b" access="read"/>
                <property name="CanPause" type="b" access="read"/>
                <property name="CanGoPrevious" type="b" access="read"/>
                <property name="CanGoNext" type="b" access="read"/>
            </interface></node>"#;
            let info = gio::DBusNodeInfo::for_xml(xml).unwrap();
            let read = data.clone();
            let all_properties = data.clone();
            let recorded = calls.clone();
            let registration = connection
                .register_object(PATH, &info.lookup_interface(PLAYER).unwrap())
                .property(move |_, _, _, _, name| read.borrow()[name].clone())
                .method_call(move |_, _, _, interface, method, _, invocation| {
                    if interface == Some("org.freedesktop.DBus.Properties") && method == "GetAll" {
                        invocation
                            .return_value(Some(&(all_properties.borrow().clone(),).to_variant()));
                    } else {
                        recorded.borrow_mut().push(method.into());
                        invocation.return_value(Some(&().to_variant()));
                    }
                })
                .build()
                .unwrap();
            let (tx, rx) = async_channel::bounded(1);
            let subscriptions = subscribe(&connection, &tx);
            connection
                .call_future(
                    Some(BUS),
                    "/org/freedesktop/DBus",
                    BUS,
                    "RequestName",
                    Some(&(name, 0u32).to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    2000,
                )
                .await
                .unwrap();
            glib::future_with_timeout(std::time::Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap();
            let players = scan(&connection).await.unwrap();
            let selected = select(players, None).unwrap();
            assert_eq!(selected.title, "Track <one> & two");
            assert_eq!(selected.artist, "Artist");
            control(&connection, name, "PlayPause").await.unwrap();
            control(&connection, name, "Next").await.unwrap();
            assert_eq!(*calls.borrow(), ["PlayPause", "Next"]);
            for status in ["Paused", "Stopped"] {
                data.borrow_mut()
                    .insert("PlaybackStatus".into(), status.to_variant());
                connection
                    .emit_signal(
                        None,
                        PATH,
                        "org.freedesktop.DBus.Properties",
                        "PropertiesChanged",
                        Some(&(PLAYER, data.borrow().clone(), Vec::<String>::new()).to_variant()),
                    )
                    .unwrap();
                glib::future_with_timeout(std::time::Duration::from_secs(2), rx.recv())
                    .await
                    .unwrap()
                    .unwrap();
                let selected = select(scan(&connection).await.unwrap(), Some(name));
                if status == "Paused" {
                    assert_eq!(selected.unwrap().playback, Playback::Paused);
                } else {
                    assert!(selected.is_none());
                }
            }
            connection
                .call_future(
                    Some(BUS),
                    "/org/freedesktop/DBus",
                    BUS,
                    "ReleaseName",
                    Some(&(name,).to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    2000,
                )
                .await
                .unwrap();
            glib::future_with_timeout(std::time::Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(scan(&connection).await.unwrap().is_empty());
            assert!(control(&connection, name, "PlayPause").await.is_err());
            drop(subscriptions);
            connection
                .emit_signal(
                    None,
                    PATH,
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    Some(&(PLAYER, data.borrow().clone(), Vec::<String>::new()).to_variant()),
                )
                .unwrap();
            connection.flush_future().await.unwrap();
            assert!(
                glib::future_with_timeout(std::time::Duration::from_millis(50), rx.recv())
                    .await
                    .is_err(),
                "dropping subscriptions must stop media refresh events"
            );
            connection.unregister_object(registration).unwrap();
        });
    }

    pub(crate) fn assert_widgets() {
        let media = Media::new();
        assert!(!media.widget.is_visible());
        let player = parse_player("org.mpris.MediaPlayer2.test", &properties("Playing")).unwrap();
        media.render(Some(player));
        assert!(media.widget.is_visible());
        assert_eq!(media.title.label().text(), "Artist - Track <one> & two");
        assert!(!media.title.label().uses_markup());
        crate::marquee::assert_one_pixel_overflow(&media.title);
        assert_eq!(
            media.play.icon_name().as_deref(),
            Some("media-playback-pause-symbolic")
        );
        assert!(!media.previous.is_sensitive());
        let player = parse_player("org.mpris.MediaPlayer2.test", &properties("Paused")).unwrap();
        media.render(Some(player));
        assert!(media.widget.is_visible());
        assert_eq!(
            media.play.icon_name().as_deref(),
            Some("media-playback-start-symbolic")
        );
        assert!(media.play.is_sensitive());
        media.render(None);
        assert!(!media.widget.is_visible());
    }
}

#[cfg(test)]
pub(crate) use tests::assert_widgets;
