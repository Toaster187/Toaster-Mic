//! `libtoaster_ui.so`: the Slint user interface (UI process only). Talks to the
//! `:audio` process through intents and reads its status file.

#[cfg(target_os = "android")]
mod java;

/// Slint's prebuilt Skia was compiled against a newer libc++ that hashes
/// strings through `std::__hash_memory`; NDK r28's libc++ predates it. Skia
/// only needs a process-wide consistent hash, so FNV-1a (64 bit) suffices.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn _ZNSt6__ndk113__hash_memoryEPKvm(ptr: *const u8, len: usize) -> usize {
    let bytes = if ptr.is_null() { &[][..] } else { unsafe { std::slice::from_raw_parts(ptr, len) } };
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h as usize
}

#[cfg(target_os = "android")]
mod ui {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::Duration;

    use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
    use toaster_android_audio::capture::{self, CaptureConfig};
    use toaster_android_audio::device::{self, BuildInfo, InputDevice};
    use toaster_android_audio::settings::Calibration;
    use toaster_android_audio::status::{flag, EngineLock, Status, StatusCode};
    use toaster_android_audio::{Mode, PhoneSettings};
    use toaster_core::calibration::{identify, LevelMeter};

    use crate::java::{self, Java};

    slint::include_modules!();

    struct Cal {
        channels: usize,
        step: usize,
        background: Vec<f64>,
        /// top, bottom, rear
        mapping: [i32; 3],
        done: bool,
    }

    struct State {
        ui: AppWindow,
        java: Java,
        files: PathBuf,
        settings: PhoneSettings,
        build: BuildInfo,
        devices: Vec<InputDevice>,
        builtin_mics: usize,
        status: Status,
        pending_start: bool,
        asked_dnd: bool,
        cal: Option<Cal>,
        tick: u32,
        /// After sending START/STOP the engine needs a moment; until then the
        /// button shows the requested state and further taps are ignored.
        transition_until: Option<std::time::Instant>,
        requested_running: bool,
    }

    thread_local! {
        static STATE: RefCell<Option<Rc<RefCell<State>>>> = const { RefCell::new(None) };
    }

    fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
        let s = STATE.with(|s| s.borrow().clone())?;
        let mut g = s.try_borrow_mut().ok()?;
        Some(f(&mut g))
    }

    fn mode_index(m: Mode) -> i32 {
        match m {
            Mode::Mono => 0,
            Mode::Stereo => 1,
            Mode::Studio => 2,
        }
    }

    #[no_mangle]
    fn android_main(app: slint::android::AndroidApp) {
        let java = match Java::new(&app) {
            Ok(j) => j,
            Err(_) => return,
        };
        if slint::android::init(app).is_err() {
            return;
        }
        let ui = AppWindow::new().expect("UI");
        let files = java.files_dir();
        let settings = PhoneSettings::load(&files);
        let build = java.build();
        let devices = java.devices();
        let builtin_mics = java.builtin_microphones();
        let state = Rc::new(RefCell::new(State {
            ui: ui.clone_strong(),
            java,
            files,
            settings,
            build,
            devices,
            builtin_mics,
            status: Status::default(),
            pending_start: false,
            asked_dnd: false,
            cal: None,
            tick: 0,
            transition_until: None,
            requested_running: false,
        }));
        STATE.with(|s| *s.borrow_mut() = Some(state));
        with(|s| s.init_ui());

        ui.on_start_stop(|| {
            with(|s| s.start_stop());
        });
        ui.on_toggle_mute(|| {
            with(|s| s.toggle_mute());
        });
        ui.on_set_mode(|m| {
            with(|s| s.set_mode(m));
        });
        ui.on_set_mono_mic(|v| {
            with(|s| s.set_mono_mic(v));
        });
        ui.on_set_live(|v| {
            with(|s| s.set_live(v));
        });
        ui.on_set_always_on(|v| {
            with(|s| s.set_always_on(v));
        });
        ui.on_open_settings(|| {
            with(|s| s.ui.set_page(1));
        });
        ui.on_close_settings(|| {
            with(|s| s.close_settings());
        });
        ui.on_gain_edited(|v| {
            with(|s| s.set_gain(v));
        });
        ui.on_left_edited(|v| {
            with(|s| s.set_left(v));
        });
        ui.on_calibrate(|| {
            with(|s| s.open_calibration());
        });
        ui.on_cal_measure(|| {
            with(|s| s.measure());
        });
        ui.on_cal_close(|| {
            with(|s| {
                s.cal = None;
                s.ui.set_page(1);
            });
        });
        ui.on_open_dnd(|| {
            with(|s| s.java.open_dnd());
        });
        ui.on_open_battery(|| {
            with(|s| s.java.open_battery());
        });

        // Polls the engine status only while this window exists.
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(250), || {
            with(|s| s.refresh());
        });
        let _ = ui.run();
        drop(timer);
        // Free everything: the UI process does not linger in the background.
        std::process::exit(0);
    }

    impl State {
        fn init_ui(&mut self) {
            let s = &self.settings;
            self.ui.set_mode(mode_index(s.mode));
            self.ui.set_mono_mic(s.mono_mic as i32);
            self.ui.set_live(s.live);
            self.ui.set_always_on(s.always_on);
            self.ui.set_muted(s.muted);
            self.ui.set_port(s.port.to_string().into());
            self.ui.set_gain(s.gain as f32);
            self.ui.set_profile(s.profile as i32);
            self.ui.set_left_index(s.left as i32);
            let mut names: Vec<SharedString> = vec!["Automatisch (Android)".into()];
            names.extend(self.devices.iter().map(|d| SharedString::from(d.label())));
            let index = self.devices.iter().position(|d| d.id == s.device).map_or(0, |i| i + 1);
            self.ui.set_inputs(ModelRc::new(VecModel::from(names)));
            self.ui.set_input_index(index as i32);
            self.ui.set_can_calibrate(self.calibration_channels() >= 2);
            self.ui.set_can_choose_mic(self.calibration_channels() >= 2);
            self.ui.set_has_rear(self.calibration_channels() >= 3);
            let names: Vec<SharedString> = if self.build.is_pixel9() {
                vec!["Mikrofon unten".into(), "Mikrofon oben".into()]
            } else {
                vec!["Mikrofon 1 (unten)".into(), "Mikrofon 2 (oben)".into()]
            };
            self.ui.set_channel_names(ModelRc::new(VecModel::from(names)));
            self.refresh();
            // After a phone restart Always-On needs one start from the foreground.
            if self.settings.always_on && self.status.code == StatusCode::Stopped && self.java.can_record() {
                self.java.send(java::STANDBY, "", 0);
            }
        }

        fn calibration_channels(&self) -> usize {
            if self.build.is_pixel9() {
                3
            } else if self.builtin_mics >= 2 {
                2
            } else {
                0
            }
        }

        fn save(&mut self) {
            if let Err(e) = self.settings.save(&self.files) {
                self.ui.set_settings_error(format!("Speichern fehlgeschlagen: {e}").into());
            }
        }

        fn engine_running(&self) -> bool {
            matches!(self.status.code, StatusCode::Starting | StatusCode::Ready | StatusCode::Streaming | StatusCode::Standby)
        }

        fn capturing(&self) -> bool {
            matches!(self.status.code, StatusCode::Starting | StatusCode::Ready | StatusCode::Streaming)
        }

        fn refresh(&mut self) {
            if let Some(mut st) = Status::read(&self.files) {
                // A killed engine leaves its last status behind.
                if st.code != StatusCode::Stopped && st.code != StatusCode::Error && !EngineLock::engine_running(&self.files) {
                    st = Status { code: StatusCode::Stopped, ..Default::default() };
                }
                self.status = st;
            }
            let st = &self.status;
            let mut running = self.capturing();
            if let Some(until) = self.transition_until {
                if running == self.requested_running || std::time::Instant::now() > until {
                    self.transition_until = None;
                } else {
                    running = self.requested_running;
                }
            }
            self.ui.set_running(running);
            let text = match st.code {
                StatusCode::Stopped if st.message.is_empty() => "Bereit".to_string(),
                _ if st.message.is_empty() => "…".to_string(),
                _ => st.message.clone(),
            };
            if self.transition_until.is_none() {
                self.ui.set_status(text.into());
            }
            self.ui.set_status_error(st.code == StatusCode::Error);
            let peer = std::net::Ipv4Addr::from(st.peer);
            let detail = match st.code {
                StatusCode::Streaming if st.link_age_ms >= 1500 => {
                    format!("PC {peer} · antwortet verzögert (letztes Signal vor {:.1} s)", st.link_age_ms as f32 / 1000.0).replace('.', ",")
                }
                StatusCode::Streaming => format!("PC {peer} · Verbindung aktiv"),
                StatusCode::Ready => "Noch kein PC verbunden. Am PC die Handy-IP eintragen und verbinden.".into(),
                StatusCode::Standby => "Mikrofon aus · der gekoppelte PC weckt es bei Bedarf.".into(),
                _ => String::new(),
            };
            self.ui.set_detail(detail.into());
            // Mute toggled from the notification: keep the app setting in sync.
            if running {
                let engine_muted = st.flags & flag::MUTED != 0;
                if engine_muted != self.settings.muted {
                    self.settings.muted = engine_muted;
                    let _ = self.settings.save(&self.files);
                }
            }
            self.ui.set_meter(st.peak as f32 / 32768.0);
            self.ui.set_left_meter(st.left_peak as f32 / 32768.0);
            self.ui.set_right_meter(st.right_peak as f32 / 32768.0);
            self.ui.set_muted(self.settings.muted);
            if self.tick % 8 == 0 {
                let ips = device::local_ipv4();
                self.ui.set_ip(if ips.is_empty() { "Keine lokale IPv4 · WLAN verbinden".into() } else { ips.join("\n").into() });
                self.ui.set_dnd_granted(self.java.dnd_access());
            }
            self.tick = self.tick.wrapping_add(1);
            let hint = if self.settings.live && !self.java.dnd_access() {
                "Für Live in den Einstellungen den Zugriff auf „Nicht stören“ erlauben."
            } else if self.settings.always_on && st.code == StatusCode::Standby && st.flags & flag::PAIRED == 0 {
                "Always-On: einmal manuell starten und am PC verbinden – danach weckt der PC das Handy selbst."
            } else {
                ""
            };
            self.ui.set_hint(hint.into());
            if self.pending_start && self.java.can_record() {
                self.pending_start = false;
                self.start();
            }
        }

        fn start_stop(&mut self) {
            // Ignore repeated taps while the engine is starting or stopping.
            if self.transition_until.is_some_and(|t| std::time::Instant::now() < t) {
                return;
            }
            if self.capturing() {
                self.transition(false);
                // Manual stop. With Always-On the phone returns to standby.
                self.java.send(if self.settings.always_on { java::STANDBY } else { java::STOP }, "", 0);
                return;
            }
            if !self.java.can_record() {
                self.pending_start = true;
                self.java.request_permissions();
                self.ui.set_status("Mikrofonberechtigung erforderlich.".into());
                return;
            }
            self.start();
        }

        fn needs_calibration(&self) -> bool {
            let mono_choice = self.settings.mode == Mode::Mono && self.settings.mono_mic > 0;
            if self.settings.mode != Mode::Stereo && !mono_choice {
                return false;
            }
            let internal = self.devices.iter().find(|d| d.id == self.settings.device).is_none_or(|d| d.kind == device::TYPE_BUILTIN_MIC);
            let ch = self.calibration_channels();
            internal && ch >= 2 && self.settings.calibrated_for(&self.build.fingerprint, ch).is_none()
        }

        fn apply_settings_page(&mut self) -> bool {
            match self.ui.get_port().trim().parse::<u16>() {
                Ok(p) if p >= 1024 => self.settings.port = p,
                _ => {
                    self.ui.set_settings_error("Port: 1024–65535".into());
                    return false;
                }
            }
            let idx = self.ui.get_input_index();
            self.settings.device = if idx <= 0 { 0 } else { self.devices.get(idx as usize - 1).map_or(0, |d| d.id) };
            self.settings.profile = self.ui.get_profile().clamp(0, 2) as u8;
            self.settings.gain = self.ui.get_gain().round().clamp(0.0, 200.0) as u32;
            self.ui.set_settings_error("".into());
            true
        }

        fn start(&mut self) {
            if !self.apply_settings_page() {
                self.ui.set_page(1);
                return;
            }
            self.save();
            if self.needs_calibration() {
                self.open_calibration();
                return;
            }
            self.transition(true);
            self.java.send(java::START, "", 0);
        }

        fn transition(&mut self, running: bool) {
            self.requested_running = running;
            self.transition_until = Some(std::time::Instant::now() + Duration::from_secs(3));
            self.ui.set_running(running);
            self.ui.set_status(if running { "Mikrofon wird gestartet …" } else { "Wird gestoppt …" }.into());
        }

        fn set_mono_mic(&mut self, v: i32) {
            if self.capturing() {
                return;
            }
            self.settings.mono_mic = v.clamp(0, 3) as u8;
            self.ui.set_mono_mic(v);
            self.save();
            if self.status.code == StatusCode::Standby {
                self.java.send(java::STANDBY, "replan", 1);
            }
        }

        fn toggle_mute(&mut self) {
            self.settings.muted = !self.settings.muted;
            self.ui.set_muted(self.settings.muted);
            self.save();
            if self.engine_running() {
                self.java.send(java::SET, "muted", self.settings.muted as i32);
            }
        }

        fn set_mode(&mut self, m: i32) {
            if self.capturing() {
                return;
            }
            self.settings.mode = match m {
                1 => Mode::Stereo,
                2 => Mode::Studio,
                _ => Mode::Mono,
            };
            self.ui.set_mode(m);
            self.save();
            // A standby engine plans its capture at start: restart it.
            if self.status.code == StatusCode::Standby {
                self.java.send(java::STANDBY, "replan", 1);
            }
        }

        fn set_live(&mut self, v: bool) {
            self.settings.live = v;
            self.ui.set_live(v);
            self.save();
            if self.engine_running() {
                self.java.send(java::SET, "live", v as i32);
            }
            if v && !self.java.dnd_access() && !self.asked_dnd {
                self.asked_dnd = true;
                self.java.open_dnd();
            }
        }

        fn set_always_on(&mut self, v: bool) {
            self.settings.always_on = v;
            self.ui.set_always_on(v);
            self.save();
            if v && !self.engine_running() {
                if self.java.can_record() {
                    self.java.send(java::STANDBY, "", 0);
                } else {
                    self.java.request_permissions();
                }
            } else if !v && self.status.code == StatusCode::Standby {
                self.java.send(java::STOP, "", 0);
            }
        }

        fn set_gain(&mut self, v: f32) {
            self.settings.gain = v.round().clamp(0.0, 200.0) as u32;
            if self.engine_running() {
                self.java.send(java::SET, "gain", self.settings.gain as i32);
            }
        }

        fn set_left(&mut self, v: i32) {
            self.settings.left = v.clamp(0, 1) as u8;
            self.save();
            if self.engine_running() {
                self.java.send(java::SET, "left", v);
            }
        }

        fn close_settings(&mut self) {
            if self.apply_settings_page() {
                self.save();
                self.ui.set_page(0);
            }
        }

        // ---- guided microphone identification ---------------------------------

        fn prompts(channels: usize) -> Vec<&'static str> {
            let mut p = vec![
                "Ruhe\n\nHandy ruhig halten und kurz nicht sprechen.",
                "Mikrofon oben\n\nNach dem Start drei Sekunden leicht direkt über die Mikrofonöffnung an der Oberkante reiben. Nicht über den Lautsprecher.",
                "Mikrofon unten\n\nNach dem Start drei Sekunden leicht direkt über die Mikrofonöffnung unten reiben.",
            ];
            if channels >= 3 {
                p.push("Mikrofon hinten\n\nNach dem Start drei Sekunden leicht über die Mikrofonöffnung an der Kamera reiben.");
            }
            p
        }

        fn open_calibration(&mut self) {
            let channels = self.calibration_channels();
            if channels < 2 {
                return;
            }
            if self.capturing() {
                self.ui.set_status("Bitte zuerst die Übertragung stoppen.".into());
                return;
            }
            if !self.java.can_record() {
                self.java.request_permissions();
                return;
            }
            self.cal = Some(Cal { channels, step: 0, background: Vec::new(), mapping: [-1; 3], done: false });
            self.show_cal_step("Es werden nur Kanalpegel gespeichert.");
            self.ui.set_page(2);
        }

        fn show_cal_step(&mut self, feedback: &str) {
            let Some(c) = &self.cal else { return };
            let prompts = Self::prompts(c.channels);
            let total = prompts.len();
            self.ui.set_cal_prompt(format!("{} / {} · {}", c.step + 1, total, prompts[c.step.min(total - 1)]).into());
            self.ui.set_cal_feedback(feedback.into());
            self.ui.set_cal_button("Messung starten".into());
        }

        fn measure(&mut self) {
            let Some(c) = &self.cal else { return };
            if c.done {
                self.cal = None;
                self.ui.set_page(1);
                return;
            }
            let channels = c.channels;
            let device_id = if self.build.is_pixel9() {
                device::builtin(&self.devices, Some("bottom")).map_or(0, |d| d.id)
            } else {
                device::builtin(&self.devices, None).map_or(0, |d| d.id)
            };
            self.ui.set_cal_busy(true);
            self.ui.set_cal_feedback("Jetzt messen …".into());
            std::thread::spawn(move || {
                let result = measure_rms(device_id, channels);
                let _ = slint::invoke_from_event_loop(move || {
                    with(|s| s.accept_measurement(result));
                });
            });
        }

        fn accept_measurement(&mut self, result: Result<Vec<f64>, String>) {
            self.ui.set_cal_busy(false);
            let rms = match result {
                Ok(r) => r,
                Err(e) => {
                    self.ui.set_cal_feedback(e.into());
                    return;
                }
            };
            let Some(c) = self.cal.as_mut() else { return };
            if c.step == 0 {
                c.background = rms;
            } else {
                let Some(channel) = identify(&c.background, &rms) else {
                    self.ui.set_cal_feedback("Nicht eindeutig. Bitte nur direkt an der genannten Öffnung leicht reiben und wiederholen.".into());
                    return;
                };
                if c.mapping[..c.step - 1].contains(&(channel as i32)) {
                    c.step = 0;
                    c.mapping = [-1; 3];
                    self.show_cal_step("Zwei Stellen reagierten auf denselben Kanal. Bitte neu messen.");
                    return;
                }
                c.mapping[c.step - 1] = channel as i32;
            }
            c.step += 1;
            let total = Self::prompts(c.channels).len();
            if c.step < total {
                self.show_cal_step("Messung übernommen.");
                return;
            }
            let rear = if c.channels >= 3 { c.mapping[2] } else { -1 };
            let cal = Calibration { build: self.build.fingerprint.clone(), channels: c.channels, top: c.mapping[0], bottom: c.mapping[1], rear };
            if !cal.valid() {
                c.step = 0;
                self.show_cal_step("Zuordnung nicht eindeutig. Bitte neu messen.");
                return;
            }
            c.done = true;
            let channels = c.channels;
            self.settings.calibration = Some(cal);
            self.settings.device = 0;
            self.save();
            self.ui.set_cal_prompt("Fertig · oben und unten zugeordnet".into());
            self.ui.set_cal_feedback(if channels >= 3 { "Die Rückseite wird in Stereo nicht übertragen." } else { "" }.into());
            self.ui.set_cal_button("Fertig".into());
        }
    }

    /// Three seconds of raw capture (first 200 ms discarded); only RMS levels are kept.
    fn measure_rms(device_id: i32, channels: usize) -> Result<Vec<f64>, String> {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel::<Vec<i16>>();
        let config = CaptureConfig { device_id, channels: channels as i32, unprocessed: true, low_latency: false, frames_per_callback: None };
        let cap = capture::start(&config, move |s| {
            let _ = tx.send(s.to_vec());
        }, |_| {})?;
        let mut meter = LevelMeter::new(channels);
        let mut frames = 0usize;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while frames < 48_000 * 32 / 10 {
            let chunk = rx.recv_timeout(Duration::from_millis(500)).map_err(|_| "Aufnahme unterbrochen.".to_string())?;
            if frames >= 9_600 {
                meter.add(&chunk);
            }
            frames += chunk.len() / channels;
            if std::time::Instant::now() > deadline {
                return Err("Aufnahme zu langsam.".into());
            }
        }
        if device_id != 0 && cap.device_id() != device_id {
            return Err("Mikrofonroute hat gewechselt. Messung wiederholen.".into());
        }
        drop(cap);
        Ok(meter.rms())
    }
}
