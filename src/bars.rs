use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    sync::atomic::Ordering,
    thread,
    time::Duration,
};

use gtk::{
    gdk, gio, glib, prelude::*, Align, Application, ApplicationWindow, Box, Label, Orientation,
    ToggleButton,
};
use gtk4_layer_shell::{Edge, Layer, LayerShell};

use crate::{
    audio, brightness,
    config::{Config, Intervals, Module},
    gpu,
    hub::Hub,
    keyboard, media, network, notifications, panels, power, previews, selection,
    status::{self, Cpu, StatusLevel},
    tasks, tray,
    workspaces::{self, Outputs, Workspaces},
    NetworkSnapshot,
};

// Bars belong to layer-shell namespace "epic-bar" so compositor layer rules
// (blur, animations) can target them.
const NAMESPACE: &str = "epic-bar";

/* Every module some output's layout uses; services for unused modules are
 * never started (no nvidia-smi polling for a bar without a GPU module). */
fn used_modules(config: &Config) -> BTreeSet<Module> {
    let mut used: BTreeSet<_> = config
        .bar
        .left
        .iter()
        .chain(&config.bar.right)
        .copied()
        .collect();
    for output in config.outputs.values() {
        used.extend(output.left.iter().flatten().copied());
        used.extend(output.right.iter().flatten().copied());
    }
    used
}

/* Data sources shared by every output's bars. */
struct Services {
    config: Rc<Config>,
    used: BTreeSet<Module>,
    previews: previews::Previews,
    status: Rc<Hub<status::Sample>>,
    networks: Rc<Hub<NetworkSnapshot>>,
    gpu: Option<Rc<Hub<gpu::Snapshot>>>,
    audio: Option<Rc<Hub<audio::Snapshot>>>,
    notifications: Option<Rc<notifications::Notifications>>,
    tray: Option<Rc<tray::Tray>>,
    keyboard: Rc<Hub<keyboard::State>>,
    brightness: Rc<Hub<brightness::Snapshot>>,
    backlight: Option<brightness::Backlight>,
    /* Bars currently showing CPU frequency; frequency is sampled only then. */
    expanded: Rc<Cell<usize>>,
    has_battery: bool,
}

impl Services {
    fn start(config: Rc<Config>, state: &Workspaces) -> Result<Rc<Self>, String> {
        let used = used_modules(&config);
        let has_battery = used.contains(&Module::Battery)
            && crate::has_battery().unwrap_or_else(|error| {
                eprintln!("epic-bar-rs: cannot detect battery: {error}");
                false
            });
        let backlight = used
            .contains(&Module::Brightness)
            .then(|| brightness::find(std::path::Path::new("/sys/class/backlight")))
            .flatten();
        let services = Rc::new(Self {
            previews: previews::Previews::new(state),
            status: Hub::new(),
            networks: Hub::new(),
            gpu: used
                .contains(&Module::Gpu)
                .then(|| gpu::watch(Intervals::duration(config.intervals.gpu))),
            audio: used.contains(&Module::Volume).then(audio::watch),
            notifications: used.contains(&Module::Notifications).then(|| {
                notifications::Notifications::watch(Intervals::duration(
                    config.intervals.notifications,
                ))
            }),
            tray: used.contains(&Module::Tray).then(tray::Tray::start),
            keyboard: Hub::new(),
            brightness: Hub::new(),
            backlight,
            expanded: Rc::new(Cell::new(0)),
            has_battery,
            used,
            config,
        });
        services.start_status()?;
        if services.used.contains(&Module::Network) {
            services.start_network();
        }
        if services.used.contains(&Module::Keyboard) || services.used.contains(&Module::Submap) {
            services.refresh_keyboard();
        }
        Ok(services)
    }

    fn start_status(&self) -> Result<(), String> {
        let clock = status::Clock::new(&self.config.clock)?;
        let mut cpu = Cpu::new();
        let mut thermal = status::Thermal::default();
        let (hub, expanded, has_battery) =
            (self.status.clone(), self.expanded.clone(), self.has_battery);
        let (brightness, backlight) = (self.brightness.clone(), self.backlight.clone());
        let mut sample = move || {
            hub.publish(status::Sample {
                battery: has_battery.then(status::get_battery_info),
                datetime: clock.now(),
                memory: status::get_mem_info(),
                cpu_load: cpu.get_cpu_load(),
                cpu_freq: (expanded.get() > 0).then(Cpu::get_avg_freq),
                cpu_temperature: thermal.celsius(),
            });
            brightness.publish(backlight.as_ref().and_then(brightness::Backlight::percent));
        };
        sample();
        glib::timeout_add_local(
            Intervals::duration(self.config.intervals.status),
            move || {
                sample();
                glib::ControlFlow::Continue
            },
        );
        Ok(())
    }

    fn start_network(&self) {
        let (tx, rx) = async_channel::bounded::<NetworkSnapshot>(1);
        let interval = Intervals::duration(self.config.intervals.network);
        thread::spawn(move || {
            let mut reader = network::Reader::default();
            let mut previous = None;
            loop {
                let snapshot = reader.read().map_err(|error| error.to_string());
                if previous.as_ref() != Some(&snapshot) {
                    if tx.send_blocking(snapshot.clone()).is_err() {
                        return;
                    }
                    previous = Some(snapshot);
                }
                thread::sleep(interval);
            }
        });
        let hub = self.networks.clone();
        glib::spawn_future_local(async move {
            while let Ok(snapshot) = rx.recv().await {
                hub.publish(snapshot);
            }
            hub.publish(Err("Network monitor stopped".into()));
        });
    }

    /* Read keyboard layouts from Hyprland, keeping the current submap. */
    fn refresh_keyboard(&self) {
        let hub = self.keyboard.clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(keyboard::read).await;
            match result {
                Ok(Ok(mut state)) => {
                    state.submap = hub
                        .with(|current| current.map(|current| current.submap.clone()))
                        .unwrap_or_default();
                    if hub.with(|current| current != Some(&state)) {
                        hub.publish(state);
                    }
                }
                Ok(Err(error)) => eprintln!("epic-bar-rs: cannot read keyboard layouts: {error}"),
                Err(_) => eprintln!("epic-bar-rs: keyboard layout reader panicked"),
            }
        });
    }

    fn observe_keyboard(&self, event: &workspaces::HyprEvent) {
        let mut state = self
            .keyboard
            .with(|state| state.cloned())
            .unwrap_or_default();
        if state.observe(event) {
            self.keyboard.publish(state);
        }
    }

    fn configured_workspaces(&self) -> BTreeMap<String, Vec<usize>> {
        self.config
            .outputs
            .iter()
            .filter_map(|(name, output)| Some((name.clone(), output.workspaces.clone()?)))
            .collect()
    }
}

/* The workspace buttons of one top bar. */
struct Strip {
    container: Box,
    selection: selection::Selection,
}

/* One output's top bar and optional task bar. */
struct OutputBars {
    output: Option<String>,
    windows: Vec<ApplicationWindow>,
    strip: Option<Strip>,
    tasks: Option<tasks::Tasks>,
    media: Option<media::Media>,
    more: Option<ToggleButton>,
    expanded: Rc<Cell<usize>>,
}

fn layer_window(
    app: &Application,
    css_name: &str,
    monitor: &gdk::Monitor,
    edge: Edge,
) -> ApplicationWindow {
    let window = ApplicationWindow::builder()
        .application(app)
        .css_name(css_name)
        .build();
    window.init_layer_shell();
    window.set_layer(Layer::Top);
    window.set_namespace(Some(NAMESPACE));
    window.set_monitor(Some(monitor));
    window.auto_exclusive_zone_enable();
    window.set_anchor(edge, true);
    window.set_anchor(Edge::Left, true);
    window.set_anchor(Edge::Right, true);
    window.set_decorated(true);
    window
}

impl OutputBars {
    fn new(app: &Application, monitor: &gdk::Monitor, services: &Rc<Services>) -> Self {
        let output = monitor.connector().map(|name| name.to_string());
        let layout = services.config.layout(output.as_deref());
        let mut bars = Self {
            output,
            windows: Vec::new(),
            strip: None,
            tasks: None,
            media: None,
            more: None,
            expanded: services.expanded.clone(),
        };

        let main_container = Box::builder()
            .orientation(Orientation::Horizontal)
            .halign(Align::BaselineFill)
            .hexpand(true)
            .hexpand_set(true)
            .homogeneous(false)
            .spacing(4)
            .css_name("main-box")
            .build();
        let left = Box::new(Orientation::Horizontal, 4);
        let spacer = Box::builder().hexpand(true).build();
        let right = Box::new(Orientation::Horizontal, 4);
        let mut details = Details::default();
        for module in &layout.left {
            if let Some(widget) = bars.module(*module, services, &mut details) {
                left.append(&widget);
            }
        }
        for module in &layout.right {
            if let Some(widget) = bars.module(*module, services, &mut details) {
                right.append(&widget);
            }
        }
        if let Some(more) = &bars.more {
            details.connect(more, &services.expanded);
        }
        main_container.append(&left);
        main_container.append(&spacer);
        main_container.append(&right);
        let top = layer_window(app, "top-bar", monitor, Edge::Top);
        top.set_child(Some(&main_container));
        top.present();
        bars.windows.push(top);

        if layout.tasks {
            let tasks = tasks::Tasks::new(&Workspaces::new(), Some(services.previews.clone()));
            let container = Box::builder()
                .orientation(Orientation::Horizontal)
                .halign(Align::BaselineFill)
                .hexpand(true)
                .hexpand_set(true)
                .homogeneous(false)
                .css_name("main-box")
                .build();
            container.append(&crate::task_scroller(tasks.widget()));
            let bottom = layer_window(app, "bottom-bar", monitor, Edge::Bottom);
            bottom.set_resizable(true);
            bottom.set_child(Some(&container));
            bottom.present();
            bars.windows.push(bottom);
            bars.tasks = Some(tasks);
        }
        bars
    }

    fn module(
        &mut self,
        module: Module,
        services: &Rc<Services>,
        details: &mut Details,
    ) -> Option<gtk::Widget> {
        let config = &services.config;
        Some(match module {
            Module::Workspaces => {
                let strip = workspace_strip(&services.previews);
                let widget = strip.selection.widget().clone().upcast();
                self.strip = Some(strip);
                widget
            }
            Module::Media => {
                let media = media::Media::new();
                media.start();
                let widget = media.widget.clone().upcast();
                self.media = Some(media);
                widget
            }
            Module::Submap => keyboard::submap_widget(&services.keyboard).upcast(),
            Module::Keyboard => {
                keyboard::layout_widget(&services.keyboard, config.keyboard.always).upcast()
            }
            Module::Tray => tray::widget(services.tray.as_ref()?).upcast(),
            Module::More => {
                let more = ToggleButton::builder()
                    .css_name("status-reveal-button")
                    .css_classes(["bar-button"])
                    .icon_name("view-more-symbolic")
                    .tooltip_text("Show CPU frequency and memory capacity")
                    .build();
                self.more = Some(more.clone());
                more.upcast()
            }
            Module::Network => network_module(&services.networks).upcast(),
            Module::Cpu => cpu_module(&services.status, details).upcast(),
            Module::Memory => memory_module(&services.status, details).upcast(),
            Module::Gpu => gpu_module(services.gpu.as_ref()?).upcast(),
            Module::Battery if services.has_battery => battery_module(&services.status).upcast(),
            Module::Battery => return None,
            Module::Brightness => brightness::widget(
                services.backlight.clone()?,
                &services.brightness,
                config.brightness_step,
            )
            .upcast(),
            Module::Volume => audio::widget(services.audio.as_ref()?, config.volume_step).upcast(),
            Module::Notifications => services.notifications.as_ref()?.widget().upcast(),
            Module::Clock => clock_module(&services.status).upcast(),
            Module::Power => power::widget(&config.power).upcast(),
        })
    }

    fn update(&self, state: &Workspaces, outputs: &Outputs, services: &Services) {
        let view = workspaces::view(
            self.output.as_deref(),
            workspaces::workspace_count(),
            &services.configured_workspaces(),
            outputs,
        );
        if let Some(strip) = &self.strip {
            strip.container.set_tooltip_text(None);
            crate::populate_workspace_box(&strip.container, state, &view);
            strip.selection.refresh();
        }
        if let Some(tasks) = &self.tasks {
            tasks.widget().set_tooltip_text(None);
            tasks.update(&view.filter(state));
        }
    }

    fn error(&self, error: &str) {
        if let Some(strip) = &self.strip {
            strip.container.set_tooltip_text(Some(error));
        }
        if let Some(tasks) = &self.tasks {
            tasks.widget().set_tooltip_text(Some(error));
        }
    }

    fn destroy(self) {
        if let Some(media) = &self.media {
            media.stop();
        }
        if self.more.as_ref().is_some_and(ToggleButton::is_active) {
            self.expanded.set(self.expanded.get().saturating_sub(1));
        }
        for window in self.windows {
            window.destroy();
        }
    }
}

/* Labels the "more" toggle reveals in one bar. */
#[derive(Default)]
struct Details {
    cpu: Option<Label>,
    memory: Option<(Label, Label)>,
}

impl Details {
    fn connect(&self, toggle: &ToggleButton, expanded: &Rc<Cell<usize>>) {
        let placeholder = || crate::metric_label("cpu-label");
        let cpu = self.cpu.clone().unwrap_or_else(placeholder);
        let (memory, detail) = self
            .memory
            .clone()
            .unwrap_or_else(|| (placeholder(), placeholder()));
        crate::connect_status_toggle(toggle, &cpu, &memory, &detail);
        let expanded = expanded.clone();
        toggle.connect_toggled(move |toggle| {
            let count = expanded.get();
            expanded.set(if toggle.is_active() {
                count + 1
            } else {
                count.saturating_sub(1)
            });
        });
    }
}

fn workspace_strip(previews: &previews::Previews) -> Strip {
    let container = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .css_name("workspaces-container")
        .build();
    let mut hover = Vec::new();
    for tag in 1..=workspaces::workspace_count() {
        let button = crate::workspace_button(tag);
        hover.push(previews.attach(&button, previews::Anchor::Workspace(tag)));
        container.append(&button);
    }
    container.connect_unmap(move |_| {
        for preview in &hover {
            preview.hide();
        }
    });
    let selection = selection::Selection::new(&container);
    Strip {
        container,
        selection,
    }
}

fn status_group(class: &str) -> Box {
    Box::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["status-group", class])
        .hexpand(false)
        .valign(Align::Center)
        .spacing(4)
        .build()
}

fn network_module(hub: &Rc<Hub<NetworkSnapshot>>) -> Box {
    let container = Box::new(Orientation::Horizontal, 4);
    let pending = crate::network_entry(
        "Network...",
        "network-wired-symbolic",
        "Checking online network interfaces",
    );
    pending.add_css_class("pending");
    container.append(&pending);
    let weak = container.downgrade();
    let previous = RefCell::new(None);
    hub.subscribe(move |snapshot| {
        let Some(container) = weak.upgrade() else {
            return false;
        };
        crate::update_networks(&container, &mut previous.borrow_mut(), snapshot.clone());
        true
    });
    container
}

fn cpu_module(hub: &Rc<Hub<status::Sample>>, details: &mut Details) -> gtk::MenuButton {
    let group = status_group("status-cpu");
    let icon = crate::status_icon("utilities-system-monitor-symbolic");
    let load = crate::metric_label("cpu-load-label");
    let frequency = crate::metric_label("cpu-label");
    frequency.set_visible(false);
    group.append(&icon);
    group.append(&load);
    group.append(&frequency);
    details.cpu = Some(frequency.clone());
    let popup = panels::Usage::new(&group, "CPU usage", "cpu-history");
    let updater = popup.updater();
    let weak = group.downgrade();
    hub.subscribe(move |sample| {
        let Some(group) = weak.upgrade() else {
            return false;
        };
        let cpu_load = sample.cpu_load;
        load.set_text(&format!("{cpu_load:.0}%"));
        let mut detail = format!("CPU usage: {cpu_load:.1}%");
        if let Some(temperature) = sample.cpu_temperature {
            detail.push_str(&format!("\nTemperature: {temperature:.0} °C"));
        }
        if frequency.is_visible() {
            let mut text = sample.cpu_freq.clone().unwrap_or_else(|| "N/A".into());
            if let Some(temperature) = sample.cpu_temperature {
                text.push_str(&format!(" {temperature:.0}°C"));
            }
            frequency.set_text(&text);
        }
        group.set_tooltip_text(Some(&detail));
        updater.update(Some(f64::from(cpu_load)), &detail);
        crate::update_status(
            &group,
            &icon,
            StatusLevel::cpu(cpu_load),
            "utilities-system-monitor-symbolic",
        );
        true
    });
    popup.button
}

fn memory_module(hub: &Rc<Hub<status::Sample>>, details: &mut Details) -> gtk::MenuButton {
    let group = status_group("status-memory");
    let icon = crate::status_icon("drive-harddisk-symbolic");
    let label = crate::metric_label("mem-label");
    let detail_label = crate::metric_label("mem-label");
    detail_label.set_visible(false);
    group.append(&icon);
    group.append(&label);
    group.append(&detail_label);
    details.memory = Some((label.clone(), detail_label.clone()));
    let popup = panels::Usage::new(&group, "Memory usage", "memory-history");
    let updater = popup.updater();
    let weak = group.downgrade();
    hub.subscribe(move |sample| {
        let Some(group) = weak.upgrade() else {
            return false;
        };
        let memory = &sample.memory;
        label.set_text(
            &memory
                .used_percent
                .map_or_else(|| "N/A".to_string(), |used| format!("{used:.0}%")),
        );
        detail_label.set_text(&memory.string);
        let detail = format!("Memory: {}", memory.string);
        group.set_tooltip_text(Some(&detail));
        crate::update_status(
            &group,
            &icon,
            memory
                .used_percent
                .map_or(StatusLevel::Warning, StatusLevel::memory),
            "drive-harddisk-symbolic",
        );
        updater.update(memory.used_percent, &detail);
        true
    });
    popup.button
}

fn gpu_module(hub: &Rc<Hub<gpu::Snapshot>>) -> gtk::MenuButton {
    let group = status_group("status-gpu");
    let icon = crate::status_icon("video-display-symbolic");
    let label = crate::metric_label("gpu-label");
    group.append(&icon);
    group.append(&label);
    let popup = panels::Usage::new(&group, "GPU usage", "gpu-history");
    let updater = popup.updater();
    popup.button.set_visible(false);
    let weak = popup.button.downgrade();
    hub.subscribe(move |snapshot| {
        let Some(button) = weak.upgrade() else {
            return false;
        };
        match snapshot {
            None => button.set_visible(false),
            Some(Err(error)) => {
                button.set_visible(true);
                label.set_text("N/A");
                group.set_tooltip_text(Some(error));
                crate::update_status(
                    &group,
                    &icon,
                    StatusLevel::Warning,
                    "video-display-symbolic",
                );
                updater.update(None, error);
            }
            Some(Ok(sample)) => {
                button.set_visible(true);
                label.set_text(
                    &sample
                        .usage
                        .map_or_else(|| "N/A".into(), |usage| format!("{usage:.0}%")),
                );
                let detail = sample.detail();
                group.set_tooltip_text(Some(&detail));
                crate::update_status(
                    &group,
                    &icon,
                    sample
                        .usage
                        .map_or(StatusLevel::Normal, |usage| StatusLevel::cpu(usage as f32)),
                    "video-display-symbolic",
                );
                updater.update(sample.usage, &detail);
            }
        }
        true
    });
    popup.button
}

fn battery_module(hub: &Rc<Hub<status::Sample>>) -> Box {
    let group = status_group("status-battery");
    let image = crate::status_icon("battery-missing-symbolic");
    let label = crate::metric_label("battery-label");
    group.append(&image);
    group.append(&label);
    let weak = group.downgrade();
    hub.subscribe(move |sample| {
        let Some(group) = weak.upgrade() else {
            return false;
        };
        let Some(battery) = &sample.battery else {
            return true;
        };
        label.set_text(&format!("{}%", battery.capacity));
        crate::update_status(&group, &image, battery.level(), &battery.icon);
        if battery.is_charging() {
            group.add_css_class("charging");
        } else {
            group.remove_css_class("charging");
        }
        group.set_tooltip_text(Some(&battery.tooltip_text));
        true
    });
    group
}

fn clock_module(hub: &Rc<Hub<status::Sample>>) -> gtk::MenuButton {
    let date = crate::metric_label("date-label");
    let time = crate::metric_label("date-label");
    let button = crate::clock_button(&date, &time);
    let weak = button.downgrade();
    hub.subscribe(move |sample| {
        if weak.upgrade().is_none() {
            return false;
        }
        date.set_text(&sample.datetime.date);
        time.set_text(&sample.datetime.time);
        true
    });
    button
}

/* Owns every output's bars and rebuilds them when outputs change. */
pub struct Shell {
    app: Application,
    services: Rc<Services>,
    bars: RefCell<Vec<OutputBars>>,
    snapshot: RefCell<(Workspaces, Outputs)>,
    rebuild_pending: Cell<bool>,
    _hold: gio::ApplicationHoldGuard,
}

fn read_snapshot() -> Result<(Workspaces, Outputs), String> {
    let mut state = workspaces::get_workspaces()?;
    let outputs = workspaces::get_outputs().unwrap_or_else(|error| {
        eprintln!("epic-bar-rs: cannot read Hyprland outputs: {error}");
        Outputs::default()
    });
    workspaces::include_visible(&mut state, &outputs);
    Ok((state, outputs))
}

impl Shell {
    pub fn start(app: &Application, config: Rc<Config>) -> Result<(), String> {
        let (state, outputs) = read_snapshot()?;
        let services = Services::start(config, &state)?;
        let shell = Rc::new(Self {
            app: app.clone(),
            services,
            bars: RefCell::default(),
            snapshot: RefCell::new((state, outputs)),
            rebuild_pending: Cell::new(false),
            // Keep running while every output is gone (e.g. a dock unplugged).
            _hold: app.hold(),
        });
        shell.rebuild();
        let display = gdk::Display::default().ok_or("no display")?;
        let weak = Rc::downgrade(&shell);
        display.monitors().connect_items_changed(move |_, _, _, _| {
            if let Some(shell) = weak.upgrade() {
                shell.schedule_rebuild();
            }
        });
        shell.watch_events();
        // The shell lives as long as the application.
        std::mem::forget(shell);
        Ok(())
    }

    fn rebuild(&self) {
        for bars in self.bars.take() {
            bars.destroy();
        }
        let Some(display) = gdk::Display::default() else {
            return;
        };
        let monitors = display.monitors();
        let mut bars = Vec::new();
        for index in 0..monitors.n_items() {
            let Some(monitor) = monitors.item(index).and_downcast::<gdk::Monitor>() else {
                continue;
            };
            let connector = monitor.connector();
            if self
                .services
                .config
                .output(connector.as_deref())
                .is_some_and(|output| output.disabled)
            {
                continue;
            }
            bars.push(OutputBars::new(&self.app, &monitor, &self.services));
        }
        let (state, outputs) = &*self.snapshot.borrow();
        for output in &bars {
            output.update(state, outputs, &self.services);
        }
        *self.bars.borrow_mut() = bars;
    }

    /* Outputs arrive in bursts (and connector names a moment later), so
     * wait for them to settle before rebuilding. */
    fn schedule_rebuild(self: &Rc<Self>) {
        if self.rebuild_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(400), move || {
            if let Some(shell) = weak.upgrade() {
                shell.rebuild_pending.set(false);
                shell.rebuild();
            }
        });
    }

    fn watch_events(self: &Rc<Self>) {
        let (tx, rx) = async_channel::bounded::<workspaces::HyprEvent>(64);
        let preview_signals = self.services.previews.signals();
        thread::spawn(move || {
            let mut last_error = None;
            loop {
                match workspaces::event_socket() {
                    Ok(mut reader) => {
                        last_error = None;
                        preview_signals.invalidate();
                        preview_signals.connected.store(true, Ordering::Release);
                        // Catch changes missed before connecting or while disconnected.
                        if tx
                            .send_blocking(workspaces::HyprEvent::ConfigReloaded)
                            .is_err()
                        {
                            return;
                        }
                        while let Some(event) = workspaces::next_event(&mut reader) {
                            preview_signals.observe(&event);
                            if tx.send_blocking(event).is_err() {
                                return;
                            }
                        }
                    }
                    Err(error) => {
                        let message = error.to_string();
                        if last_error.as_ref() != Some(&message) {
                            eprintln!(
                                "epic-bar-rs: cannot watch Hyprland events: {message}; retrying"
                            );
                            last_error = Some(message);
                        }
                    }
                }
                preview_signals.connected.store(false, Ordering::Release);
                preview_signals.invalidate();
                thread::sleep(Duration::from_millis(500));
            }
        });

        // All bars use the same snapshot; coalesce event bursts into one refresh.
        let shell = self.clone();
        glib::spawn_future_local(async move {
            let mut attention = workspaces::Attention::default();
            while let Ok(event) = rx.recv().await {
                let mut refresh = false;
                let mut reconnected = false;
                let mut handle = |event: workspaces::HyprEvent| {
                    attention.observe(&event);
                    shell.services.observe_keyboard(&event);
                    refresh |= event.refreshes_workspaces();
                    reconnected |= event == workspaces::HyprEvent::ConfigReloaded;
                };
                handle(event);
                for _ in 0..64 {
                    match rx.try_recv() {
                        Ok(event) => handle(event),
                        Err(_) => break,
                    }
                }
                if reconnected
                    && (shell.services.used.contains(&Module::Keyboard)
                        || shell.services.used.contains(&Module::Submap))
                {
                    // Layouts can change with a config reload or a new keyboard.
                    shell.services.refresh_keyboard();
                }
                if !refresh {
                    continue;
                }
                let (mut state, outputs) = match read_snapshot() {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        eprintln!("epic-bar-rs: cannot refresh bars: {error}");
                        for bars in shell.bars.borrow().iter() {
                            bars.error(&error);
                        }
                        continue;
                    }
                };
                attention.apply(&mut state);
                shell.services.previews.update(&state);
                for bars in shell.bars.borrow().iter() {
                    bars.update(&state, &outputs, &shell.services);
                }
                *shell.snapshot.borrow_mut() = (state, outputs);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_modules_in_some_layout_start_services() {
        let mut config = Config::default();
        config.bar.right = vec![Module::Clock];
        config.outputs.insert(
            "HDMI-A-1".into(),
            crate::config::Output {
                right: Some(vec![Module::Gpu]),
                ..Default::default()
            },
        );
        let used = used_modules(&config);
        assert!(used.contains(&Module::Gpu));
        assert!(used.contains(&Module::Clock));
        assert!(used.contains(&Module::Workspaces));
        assert!(!used.contains(&Module::Volume));
    }
}
