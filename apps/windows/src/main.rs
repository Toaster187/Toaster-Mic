#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("Toaster Mic für Windows läuft nur unter Windows.");
}

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(windows)]
mod app {
    use std::cell::RefCell;
    use std::net::Ipv4Addr;
    use std::rc::Rc;
    use std::time::Duration;

    use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
    use toaster_dsp::{NoiseLevel, Studio};
    use toaster_windows::audio::{self, Endpoint};
    use toaster_windows::monitor::{self, AnnounceListener, MicMonitor};
    use toaster_windows::receiver::{Config, Receiver, WakeConfig};
    use toaster_windows::settings::{Pairing, Settings};
    use toaster_windows::{icon_rgba, system};
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    slint::include_modules!();

    /// Always-On keeps the phone awake this long after the last program
    /// stopped recording, so short pauses do not restart the capture.
    const RELEASE_DELAY: Duration = Duration::from_secs(5);

    struct Tray {
        _icon: TrayIcon,
        mute: CheckMenuItem,
        connect: MenuItem,
    }

    struct App {
        ui: MainWindow,
        settings: Settings,
        devices: Vec<Endpoint>,
        receiver: Option<Receiver>,
        auto_started: bool,
        busy: bool,
        mic_in_use: bool,
        quitting: bool,
        ui_timer: Timer,
        tick_timer: Timer,
        release_timer: Timer,
        monitor: Option<MicMonitor>,
        announce: Option<AnnounceListener>,
        tray: Option<Tray>,
    }

    thread_local! {
        static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
    }

    fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
        let app = APP.with(|a| a.borrow().clone())?;
        let mut guard = app.try_borrow_mut().ok()?;
        Some(f(&mut guard))
    }

    /// Run `f` on the UI thread with the app state.
    fn post(f: impl FnOnce(&mut App) + Send + 'static) {
        let _ = slint::invoke_from_event_loop(move || {
            with(f);
        });
    }

    pub fn run() {
        let start_hidden = std::env::args().any(|a| a == "--tray");
        if !system::claim_single_instance() {
            return;
        }
        let settings = Settings::load();
        let ui = MainWindow::new().expect("Fenster");
        let icon = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(&icon_rgba(64), 64, 64));
        ui.set_app_icon(icon);
        ui.set_host(settings.host.clone().into());
        ui.set_gain(settings.gain as f32);
        ui.set_gain_text(gain_text(settings.gain).into());
        ui.set_muted(settings.muted);
        ui.set_always_on_badge(settings.always_on);

        let app = Rc::new(RefCell::new(App {
            ui: ui.clone_strong(),
            settings,
            devices: Vec::new(),
            receiver: None,
            auto_started: false,
            busy: false,
            mic_in_use: false,
            quitting: false,
            ui_timer: Timer::default(),
            tick_timer: Timer::default(),
            release_timer: Timer::default(),
            monitor: None,
            announce: None,
            tray: None,
        }));
        APP.with(|a| *a.borrow_mut() = Some(app.clone()));

        ui.on_connect_clicked(|| {
            with(|a| a.toggle_connection());
        });
        ui.on_open_settings(|| {
            with(|a| a.open_settings());
        });
        ui.on_save_settings(|| {
            with(|a| a.save_settings_page());
        });
        ui.on_cancel_settings(|| {
            with(|a| a.ui.set_settings_page(false));
        });
        ui.on_gain_edited(|v| {
            with(|a| a.set_gain(v as f64));
        });
        ui.on_mute_edited(|m| {
            with(|a| a.set_muted(m));
        });
        ui.on_unpair(|| {
            with(|a| a.unpair());
        });
        ui.window().on_close_requested(|| {
            with(|a| a.hide_window());
            slint::CloseRequestResponse::HideWindow
        });

        with(|a| {
            a.tray = build_tray(a.settings.muted);
            a.refresh_devices(false);
            if a.settings.always_on {
                a.start_always_on();
            }
            if a.settings.auto_connect && !a.settings.always_on {
                a.connect(false);
            }
        });
        system::on_second_instance(|| post(|a| a.show_window()));
        install_tray_handlers();

        if !start_hidden {
            with(|a| a.show_window());
        }
        let _ = slint::run_event_loop_until_quit();
        with(|a| a.shutdown_blocking());
    }

    fn build_tray(muted: bool) -> Option<Tray> {
        let menu = Menu::new();
        let open = MenuItem::with_id("open", "Öffnen", true, None);
        let connect = MenuItem::with_id("connect", "Verbinden", true, None);
        let mute = CheckMenuItem::with_id("mute", "Stumm", true, muted, None);
        let quit = MenuItem::with_id("quit", "Beenden", true, None);
        menu.append_items(&[&open, &connect, &mute, &PredefinedMenuItem::separator(), &quit]).ok()?;
        let icon = tray_icon::Icon::from_rgba(icon_rgba(32), 32, 32).ok()?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("Toaster Mic")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .ok()?;
        Some(Tray { _icon: tray, mute, connect })
    }

    fn install_tray_handlers() {
        MenuEvent::set_event_handler(Some(|event: MenuEvent| {
            let id = event.id.0.clone();
            post(move |a| match id.as_str() {
                "open" => a.show_window(),
                "connect" => a.toggle_connection(),
                "mute" => {
                    let m = !a.settings.muted;
                    a.set_muted(m);
                    a.ui.set_muted(m);
                }
                "quit" => a.quit(),
                _ => {}
            });
        }));
        TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                post(|a| a.show_window());
            }
        }));
    }

    fn gain_text(v: f64) -> String {
        if v <= 0.0 {
            return "0 %".into();
        }
        let db = 20.0 * (v / 100.0).log10();
        let db = if db.abs() < 0.05 { "0 dB".to_string() } else { format!("{db:+.1} dB").replace('.', ",") };
        format!("{v:.0} % · {db}")
    }

    impl App {
        fn show_window(&mut self) {
            if self.ui.window().is_visible() {
                return;
            }
            let _ = self.ui.show();
            self.refresh_ui();
            self.ui_timer.start(TimerMode::Repeated, Duration::from_millis(150), || {
                with(|a| a.refresh_ui());
            });
        }

        fn hide_window(&mut self) {
            // Rendering stops entirely while only the tray icon is visible.
            self.ui_timer.stop();
            self.ui.set_settings_page(false);
            self.save();
        }

        fn quit(&mut self) {
            self.quitting = true;
            self.ui_timer.stop();
            let _ = self.ui.hide();
            let _ = slint::quit_event_loop();
        }

        fn shutdown_blocking(&mut self) {
            self.save();
            self.monitor = None;
            if let Some(r) = self.receiver.take() {
                let done = r.shutdown();
                let deadline = std::time::Instant::now() + Duration::from_millis(1500);
                while !done.is_finished() && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }

        fn save(&mut self) {
            self.settings.host = self.ui.get_host().trim().to_string();
            self.settings.gain = self.ui.get_gain() as f64;
            self.settings.save();
        }

        fn set_gain(&mut self, v: f64) {
            self.settings.gain = v;
            self.ui.set_gain_text(gain_text(v).into());
            if let Some(r) = &self.receiver {
                r.set_gain(v as f32 / 100.0);
            }
        }

        fn set_muted(&mut self, m: bool) {
            self.settings.muted = m;
            if let Some(r) = &self.receiver {
                r.set_muted(m);
            }
            if let Some(t) = &self.tray {
                t.mute.set_checked(m);
            }
            self.settings.save();
        }

        fn set_busy(&mut self, busy: bool) {
            self.busy = busy;
            self.ui.set_busy(busy);
            self.update_connection_labels();
        }

        fn update_connection_labels(&self) {
            let connected = self.receiver.is_some();
            self.ui.set_connected(connected);
            if let Some(t) = &self.tray {
                t.connect.set_text(if connected { "Trennen" } else { "Verbinden" });
                t.connect.set_enabled(!self.busy);
            }
        }

        fn refresh_devices(&mut self, then_open_settings: bool) {
            std::thread::spawn(move || {
                let found = audio::render_endpoints().unwrap_or_default();
                post(move |a| {
                    a.devices = found;
                    let selected = a
                        .devices
                        .iter()
                        .find(|d| Some(&d.id) == a.settings.output.as_ref())
                        .or_else(|| a.devices.iter().find(|d| audio::is_cable_input(&d.name)))
                        .cloned();
                    a.settings.output = selected.as_ref().map(|d| d.id.clone());
                    a.ui.set_output_name(
                        selected.map_or("Audioziel in den Einstellungen auswählen.".into(), |d| d.name).into(),
                    );
                    if then_open_settings {
                        a.fill_settings_page();
                    }
                });
            });
        }

        fn open_settings(&mut self) {
            if self.busy || self.receiver.is_some() {
                return;
            }
            self.refresh_devices(true);
        }

        fn fill_settings_page(&mut self) {
            let names: Vec<SharedString> = self.devices.iter().map(|d| d.name.clone().into()).collect();
            let index = self.devices.iter().position(|d| Some(&d.id) == self.settings.output.as_ref()).map_or(-1, |i| i as i32);
            self.ui.set_devices(ModelRc::new(VecModel::from(names)));
            self.ui.set_device_index(index);
            self.ui.set_port(self.settings.port.to_string().into());
            self.ui.set_buffer_index(self.settings.buffer as i32);
            self.ui.set_noise_index(self.settings.noise as i32);
            self.ui.set_always_on(self.settings.always_on);
            self.ui.set_autostart(self.settings.autostart);
            self.ui.set_auto_connect(self.settings.auto_connect);
            self.ui.set_pairing_text(self.pairing_text().into());
            self.ui.set_settings_error("".into());
            self.ui.set_settings_page(true);
        }

        fn pairing_text(&self) -> String {
            match &self.settings.pairing {
                Some(p) if p.acked => "Mit dem Handy gekoppelt".into(),
                Some(_) => "Kopplung beim nächsten Verbinden".into(),
                None => "Nicht gekoppelt".into(),
            }
        }

        fn save_settings_page(&mut self) {
            let port = match self.ui.get_port().trim().parse::<u16>() {
                Ok(p) if p >= 1024 => p,
                _ => {
                    self.ui.set_settings_error("Port: 1024–65535".into());
                    return;
                }
            };
            let index = self.ui.get_device_index();
            let Some(device) = (index >= 0).then(|| self.devices.get(index as usize)).flatten().cloned() else {
                self.ui.set_settings_error("Audioziel auswählen.".into());
                return;
            };
            let autostart = self.ui.get_autostart();
            if autostart != self.settings.autostart {
                if let Err(e) = system::set_autostart(autostart) {
                    self.ui.set_settings_error(format!("Autostart: {}", e.message()).into());
                    return;
                }
            }
            self.settings.port = port;
            self.settings.output = Some(device.id.clone());
            self.settings.buffer = self.ui.get_buffer_index().clamp(0, 3) as usize;
            self.settings.noise = self.ui.get_noise_index().clamp(0, 2) as u8;
            self.settings.autostart = autostart;
            self.settings.auto_connect = self.ui.get_auto_connect();
            let always_on = self.ui.get_always_on();
            if always_on != self.settings.always_on {
                self.settings.always_on = always_on;
                if always_on {
                    self.start_always_on();
                } else {
                    self.stop_always_on();
                }
            }
            self.ui.set_always_on_badge(always_on);
            self.ui.set_output_name(device.name.into());
            self.save();
            self.ui.set_settings_page(false);
        }

        fn unpair(&mut self) {
            self.settings.pairing = None;
            if let Some(l) = &self.announce {
                l.set_pairing(None);
            }
            self.settings.save();
            self.ui.set_pairing_text(self.pairing_text().into());
        }

        fn ensure_pairing(&mut self) -> &Pairing {
            if self.settings.pairing.as_ref().and_then(|p| p.secret_bytes()).is_none() {
                self.settings.pairing = Some(Pairing::generate());
                self.settings.save();
            }
            self.settings.pairing.as_ref().unwrap()
        }

        fn start_always_on(&mut self) {
            let pairing = self.ensure_pairing().clone();
            if self.announce.is_none() {
                self.announce = AnnounceListener::start(|ip, port, counter| {
                    post(move |a| a.on_announce(ip, port, counter));
                })
                .ok();
            }
            if let (Some(l), Some(secret)) = (&self.announce, pairing.secret_bytes()) {
                l.set_pairing(Some((secret, pairing.announce_counter)));
            }
            if self.monitor.is_none() {
                self.monitor = monitor::watch(|in_use| post(move |a| a.on_mic(in_use))).ok();
            }
        }

        fn stop_always_on(&mut self) {
            self.monitor = None;
            if let Some(l) = &self.announce {
                l.set_pairing(None);
            }
            self.mic_in_use = false;
            self.release_timer.stop();
        }

        fn on_mic(&mut self, in_use: bool) {
            self.mic_in_use = in_use;
            if !self.settings.always_on || self.quitting {
                return;
            }
            if in_use {
                self.release_timer.stop();
                if self.receiver.is_none() && !self.busy {
                    self.connect(true);
                }
            } else if self.receiver.is_some() && self.auto_started {
                self.release_timer.start(TimerMode::SingleShot, RELEASE_DELAY, || {
                    with(|a| {
                        if !a.mic_in_use && a.auto_started && a.receiver.is_some() {
                            a.disconnect();
                        }
                    });
                });
            }
        }

        fn on_announce(&mut self, ip: Ipv4Addr, port: u16, counter: u64) {
            if let Some(p) = self.settings.pairing.as_mut() {
                p.announce_counter = counter;
            }
            if port >= 1024 {
                self.settings.port = port;
            }
            let host = ip.to_string();
            if self.settings.host != host {
                self.settings.host = host.clone();
                self.ui.set_host(host.into());
            }
            self.settings.save();
            if self.settings.always_on && self.mic_in_use && self.receiver.is_none() && !self.busy {
                self.connect(true);
            }
        }

        fn toggle_connection(&mut self) {
            if self.busy {
                return;
            }
            if self.receiver.is_some() {
                self.disconnect();
            } else {
                self.connect(false);
            }
        }

        fn connect(&mut self, auto: bool) {
            let host_text = self.ui.get_host().trim().to_string();
            let host: Ipv4Addr = match host_text.parse() {
                Ok(h) if h != Ipv4Addr::UNSPECIFIED && h != Ipv4Addr::BROADCAST => h,
                _ => {
                    self.ui.set_status("Gültige Handy-IP eingeben.".into());
                    return;
                }
            };
            let Some(device) = self.settings.output.clone() else {
                self.ui.set_status("In den Einstellungen CABLE Input auswählen.".into());
                return;
            };
            let pairing = self.settings.always_on.then(|| self.ensure_pairing().clone());
            let secret = pairing.as_ref().and_then(|p| p.secret_bytes());
            let wake = match (&pairing, secret) {
                (Some(p), Some(secret)) if auto && p.acked => {
                    // Reserve a block of counters so retries never repeat one.
                    let counter = p.counter;
                    if let Some(p) = self.settings.pairing.as_mut() {
                        p.counter += 1000;
                    }
                    self.settings.save();
                    Some(WakeConfig { secret, counter })
                }
                _ => None,
            };
            let pair = secret.map(|s| (s, pairing.as_ref().map_or(0, |p| p.counter)));
            let config = Config {
                host,
                port: self.settings.port,
                target_ms: self.settings.buffer_ms(),
                gain: self.settings.gain as f32 / 100.0,
                muted: self.settings.muted,
                wake,
                pair,
                studio: Some(Box::new(Studio::new(NoiseLevel::from_index(self.settings.noise)))),
            };
            self.settings.host = host_text;
            self.ui.set_status(if auto { "Mikrofon wird verwendet · wecke Handy …" } else { "Verbinde …" }.into());
            self.set_busy(true);
            std::thread::spawn(move || {
                let result = Receiver::start(config, audio::output_factory(device));
                post(move |a| a.on_started(result, auto));
            });
        }

        fn on_started(&mut self, result: std::io::Result<Receiver>, auto: bool) {
            match result {
                Ok(r) if self.quitting => {
                    let _ = r.shutdown();
                }
                Ok(r) => {
                    self.receiver = Some(r);
                    self.auto_started = auto;
                    self.tick_timer.start(TimerMode::Repeated, Duration::from_secs(1), || {
                        with(|a| a.background_tick());
                    });
                    self.settings.save();
                }
                Err(e) => self.ui.set_status(e.to_string().into()),
            }
            self.set_busy(false);
            self.refresh_ui();
            // The program may have released the microphone while we connected.
            if auto && !self.mic_in_use {
                self.on_mic(false);
            }
        }

        fn disconnect(&mut self) {
            let Some(r) = self.receiver.take() else { return };
            self.tick_timer.stop();
            self.release_timer.stop();
            self.auto_started = false;
            self.set_busy(true);
            self.ui.set_status("Trenne …".into());
            let done = r.shutdown();
            std::thread::spawn(move || {
                let _ = done.join();
                post(|a| {
                    a.set_busy(false);
                    a.ui.set_status("Getrennt".into());
                    a.ui.set_stats("Keine Übertragung".into());
                    a.ui.set_details("".into());
                    a.ui.set_meter(0.0);
                    if a.settings.always_on && a.mic_in_use {
                        a.connect(true);
                    }
                });
            });
        }

        /// Runs once per second while connected, also with the window hidden.
        fn background_tick(&mut self) {
            let Some(r) = &self.receiver else { return };
            if r.paired() {
                if let Some(p) = self.settings.pairing.as_mut() {
                    if !p.acked {
                        p.acked = true;
                        self.settings.save();
                        self.ui.set_pairing_text(self.pairing_text().into());
                    }
                }
            }
        }

        fn refresh_ui(&mut self) {
            let Some(r) = &self.receiver else {
                self.update_connection_labels();
                return;
            };
            let s = r.snapshot();
            self.ui.set_status(r.status().into());
            self.ui.set_stats(format!("Puffer {} ms · Ziel {} ms", s.stats.buffered_ms, s.target_ms).into());
            self.ui.set_details(
                format!(
                    "Pakete {} · Fehlend {} · Zu spät {} · Neusynchronisiert {} · In Sprechpausen gekürzt {} ms",
                    s.stats.received, s.stats.lost, s.stats.late, s.stats.resyncs, s.trimmed_ms
                )
                .into(),
            );
            self.ui.set_meter(s.peak as f32 / 32768.0);
            self.update_connection_labels();
        }
    }
}
