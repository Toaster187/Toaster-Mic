//! Engine of the `:audio` process: control thread (poll on socket + eventfd,
//! no timer wake-ups in standby), capture lifecycle, status file, pairing.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use toaster_android_audio::capture::{self, Capture, CaptureConfig};
use toaster_android_audio::device::{self, BuildInfo, InputDevice};
use toaster_android_audio::status::{flag, Status, StatusCode, StatusWriter};
use toaster_android_audio::{Mode, PairingFile, PhoneSettings};
use toaster_core::phone::{self, Event, Peer, PhoneControl, LEASE_MS};
use toaster_core::wire::{self, flags};
use toaster_core::now_ms;

use crate::sender::Sender;

pub const EVENT_CAPTURING: i32 = 1;
pub const EVENT_IDLE: i32 = 2;
pub const EVENT_STOPPED: i32 = 3;

/// Fast start after an Always-On wake-up.
const FAST_START_MS: u64 = 10_000;

pub struct Shared {
    pub stop: AtomicBool,
    pub announce: AtomicBool,
    pub muted: AtomicBool,
    pub live: AtomicBool,
    pub gain_pct: AtomicU32,
    pub left: AtomicU8,
    pub profile: AtomicU8,
    pub peer: Mutex<Option<Arc<Peer>>>,
    pub peer_gen: AtomicU64,
    pub last_hello: AtomicU64,
    pub fast_until: AtomicU64,
    pub peak: AtomicU32,
    pub left_peak: AtomicU32,
    pub right_peak: AtomicU32,
    pub packets: AtomicU32,
    pub error: Mutex<Option<String>>,
    wake_fd: i32,
}

impl Shared {
    pub fn fail(&self, message: &str) {
        if let Ok(mut e) = self.error.try_lock() {
            if e.is_none() {
                *e = Some(message.to_string());
            }
        }
        self.wake();
    }
    pub fn wake(&self) {
        let one: u64 = 1;
        unsafe {
            libc::write(self.wake_fd, &one as *const u64 as *const _, 8);
        }
    }
    fn set_peer(&self, peer: Option<Arc<Peer>>) {
        *self.peer.lock().unwrap() = peer;
        self.peer_gen.fetch_add(1, Ordering::AcqRel);
    }
}

/// What to capture and how to map it onto the transmitted channels.
#[derive(Clone)]
pub struct Plan {
    pub config: CaptureConfig,
    pub send_channels: u8,
    pub map: [[usize; 4]; 2],
    pub flags: u8,
    pub label: String,
    /// Expected routed device (Pixel raw microphones); 0 = any.
    pub expect_device: i32,
}

pub struct Setup {
    pub files: PathBuf,
    pub settings: PhoneSettings,
    pub plan: Result<Plan, String>,
    pub fallback: Plan,
}

fn mono_plan(settings: &PhoneSettings, devices: &[InputDevice], unprocessed: bool, low_latency: bool) -> Plan {
    let device_id = devices.iter().find(|d| d.id == settings.device).map_or(0, |d| d.id);
    Plan {
        config: CaptureConfig { device_id, channels: 1, unprocessed, low_latency, frames_per_callback: None },
        send_channels: 1,
        map: [[0; 4]; 2],
        flags: 0,
        label: "Mono".into(),
        expect_device: 0,
    }
}

pub fn plan(
    settings: &PhoneSettings,
    build: &BuildInfo,
    devices: &[InputDevice],
    builtin_mics: usize,
    unprocessed: bool,
) -> (Result<Plan, String>, Plan) {
    let low_latency = settings.profile == 0;
    let fallback = mono_plan(settings, devices, unprocessed, low_latency);
    let selected = devices.iter().find(|d| d.id == settings.device);
    let internal = selected.is_none_or(|d| d.kind == device::TYPE_BUILTIN_MIC);
    let raw = |device_id: i32, channels: i32| CaptureConfig { device_id, channels, unprocessed: true, low_latency, frames_per_callback: None };
    let plan = match settings.mode {
        Mode::Mono => Ok(fallback.clone()),
        Mode::Stereo if build.is_pixel9() && internal => {
            let Some(bottom) = device::builtin(devices, Some("bottom")) else {
                return (Err("Pixel-Rohmikrofone nicht verfügbar.".into()), fallback);
            };
            match settings.calibrated_for(&build.fingerprint, 3) {
                Some(c) => Ok(Plan {
                    config: raw(bottom.id, 3),
                    send_channels: 2,
                    map: [[c.bottom as usize, c.top as usize, 0, 0], [c.top as usize, c.bottom as usize, 0, 0]],
                    flags: 0,
                    label: "Stereo · oben + unten".into(),
                    expect_device: bottom.id,
                }),
                None => Err("Bitte zuerst oben, unten und hinten unter Einstellungen zuordnen.".into()),
            }
        }
        Mode::Stereo if internal => match settings.calibrated_for(&build.fingerprint, 2) {
            Some(c) => Ok(Plan {
                config: raw(selected.map_or(0, |d| d.id), 2),
                send_channels: 2,
                map: [[c.bottom as usize, c.top as usize, 0, 0], [c.top as usize, c.bottom as usize, 0, 0]],
                flags: 0,
                label: "Stereo · 2 Mikrofone".into(),
                expect_device: 0,
            }),
            None => Err("Bitte zuerst die Mikrofone unter Einstellungen zuordnen.".into()),
        },
        Mode::Stereo => Ok(Plan {
            config: raw(selected.map_or(0, |d| d.id), 2),
            send_channels: 2,
            map: [[0, 1, 0, 0], [1, 0, 0, 0]],
            flags: 0,
            label: "Stereo · externer Eingang".into(),
            expect_device: 0,
        }),
        Mode::Studio => {
            let (device, channels) = if build.is_pixel9() {
                (device::builtin(devices, Some("bottom")), 3)
            } else {
                (device::builtin(devices, None), builtin_mics.clamp(0, 4) as i32)
            };
            match device {
                Some(d) if channels >= 2 => Ok(Plan {
                    config: raw(d.id, channels),
                    send_channels: channels as u8,
                    map: [[0, 1, 2, 3], [0, 1, 2, 3]],
                    flags: flags::STUDIO,
                    label: format!("Studio · {channels} Mikrofone"),
                    expect_device: if build.is_pixel9() { d.id } else { 0 },
                }),
                _ => Err("Studio braucht mindestens zwei eingebaute Mikrofone.".into()),
            }
        }
    };
    (plan, fallback)
}

pub struct Engine {
    pub shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Engine {
    pub fn start(setup: Setup, standby: bool, notify: Arc<dyn Fn(i32, i32) + Send + Sync>) -> Result<Engine, String> {
        let wake_fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if wake_fd < 0 {
            return Err("eventfd nicht verfügbar".into());
        }
        let s = &setup.settings;
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            announce: AtomicBool::new(standby),
            muted: AtomicBool::new(s.muted),
            live: AtomicBool::new(s.live),
            gain_pct: AtomicU32::new(s.gain),
            left: AtomicU8::new(s.left),
            profile: AtomicU8::new(s.profile),
            peer: Mutex::new(None),
            peer_gen: AtomicU64::new(0),
            last_hello: AtomicU64::new(0),
            fast_until: AtomicU64::new(0),
            peak: AtomicU32::new(0),
            left_peak: AtomicU32::new(0),
            right_peak: AtomicU32::new(0),
            packets: AtomicU32::new(0),
            error: Mutex::new(None),
            wake_fd,
        });
        let socket = UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), s.port))
            .map_err(|e| format!("UDP-Port {} belegt: {e}", s.port))?;
        socket.set_nonblocking(true).map_err(|e| e.to_string())?;
        let socket = Arc::new(socket);
        let mut control = Control::new(setup, shared.clone(), socket, notify, standby)?;
        let thread = std::thread::Builder::new()
            .name("toaster-control".into())
            .spawn(move || control.run())
            .map_err(|e| e.to_string())?;
        Ok(Engine { shared, thread: Some(thread) })
    }

    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.wake();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
        unsafe {
            libc::close(self.shared.wake_fd);
        }
    }
}

struct Control {
    setup: Setup,
    shared: Arc<Shared>,
    socket: Arc<UdpSocket>,
    notify: Arc<dyn Fn(i32, i32) + Send + Sync>,
    phone: PhoneControl,
    pairing_file: PairingFile,
    always_on: bool,
    capture: Option<Capture>,
    plan_label: String,
    capture_channels: u8,
    status: StatusWriter,
    message: String,
    code: StatusCode,
}

impl Control {
    fn new(
        setup: Setup,
        shared: Arc<Shared>,
        socket: Arc<UdpSocket>,
        notify: Arc<dyn Fn(i32, i32) + Send + Sync>,
        standby: bool,
    ) -> Result<Control, String> {
        let pairing_file = PairingFile::load(&setup.files);
        let mut phone = PhoneControl::new(pairing_file.pairing());
        phone.standby = standby;
        let status = StatusWriter::open(&setup.files);
        let mut c = Control {
            always_on: standby,
            setup,
            shared,
            socket,
            notify,
            phone,
            pairing_file,
            capture: None,
            plan_label: String::new(),
            capture_channels: 0,
            status,
            message: String::new(),
            code: StatusCode::Starting,
        };
        if standby {
            c.enter_standby();
        } else {
            c.start_capture(false)?;
        }
        Ok(c)
    }

    fn profile(&self) -> i32 {
        self.shared.profile.load(Ordering::Relaxed) as i32
    }

    fn start_capture(&mut self, woken: bool) -> Result<(), String> {
        let plan = match (&self.setup.plan, woken) {
            (Ok(p), _) => p.clone(),
            (Err(_), true) => self.setup.fallback.clone(),
            (Err(e), false) => return Err(e.clone()),
        };
        let mut config = plan.config.clone();
        if woken {
            config.low_latency = true;
        }
        let capture_channels = config.channels as usize;
        let mut sender = Sender::new(self.shared.clone(), self.socket.clone(), capture_channels, plan.send_channels, plan.map, plan.flags);
        let errors = self.shared.clone();
        let capture = capture::start(&config, move |s| sender.on_audio(s), move |e| errors.fail(&format!("Mikrofon unterbrochen ({e}). Bitte erneut starten.")))?;
        if plan.expect_device != 0 && capture.device_id() != plan.expect_device {
            return Err("Kalibrierte Pixel-Mikrofonroute hat gewechselt.".into());
        }
        if woken {
            self.shared.fast_until.store(now_ms() + FAST_START_MS, Ordering::Release);
        }
        self.capture = Some(capture);
        self.plan_label = plan.label;
        self.capture_channels = plan.send_channels;
        (self.notify)(EVENT_CAPTURING, if woken { 0 } else { self.profile() });
        Ok(())
    }

    fn enter_standby(&mut self) {
        self.capture = None;
        self.shared.set_peer(None);
        self.shared.fast_until.store(0, Ordering::Release);
        self.phone.drop_peer();
        self.phone.standby = true;
        self.code = StatusCode::Standby;
        self.message = if self.phone.pairing.is_some() {
            "Always-On bereit · wartet auf den PC".into()
        } else {
            "Always-On: einmal am PC manuell verbinden, um zu koppeln".into()
        };
        self.shared.announce.store(true, Ordering::Release);
        self.write_status();
        (self.notify)(EVENT_IDLE, self.profile());
    }

    fn run(&mut self) {
        let mut rx = [0u8; 2048];
        let mut events = Vec::new();
        let mut last_status = 0u64;
        loop {
            // Idle in standby: block without timeout until a datagram or a command.
            let timeout = if self.capture.is_some() { 100 } else { -1 };
            let mut fds = [
                libc::pollfd { fd: self.socket.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: self.shared.wake_fd, events: libc::POLLIN, revents: 0 },
            ];
            unsafe {
                libc::poll(fds.as_mut_ptr(), 2, timeout);
            }
            if fds[1].revents != 0 {
                let mut v = 0u64;
                unsafe {
                    libc::read(self.shared.wake_fd, &mut v as *mut u64 as *mut _, 8);
                }
            }
            if self.shared.stop.load(Ordering::Acquire) {
                break;
            }
            let failure = self.shared.error.lock().unwrap().take();
            if let Some(e) = failure {
                self.code = StatusCode::Error;
                self.message = format!("Fehler: {e}");
                self.capture = None;
                self.write_status();
                (self.notify)(EVENT_STOPPED, 0);
                break;
            }
            if self.shared.announce.swap(false, Ordering::AcqRel) {
                self.announce();
            }
            while let Ok((n, from)) = self.socket.recv_from(&mut rx) {
                events.clear();
                let now = now_ms();
                self.phone.on_datagram(&rx[..n], from, now, &mut events);
                for e in events.drain(..) {
                    self.handle(e);
                }
                self.shared.last_hello.store(self.phone.last_hello(), Ordering::Release);
            }
            let now = now_ms();
            let active = self.phone.active(now).is_some();
            if self.always_on && self.capture.is_some() && !active && self.phone.standby_allowed(now) {
                // The PC stopped or vanished: back to the sparse standby.
                self.enter_standby();
            }
            if now.saturating_sub(last_status) >= 100 || self.capture.is_none() {
                last_status = now;
                self.refresh_status(now);
            }
        }
        self.capture = None;
        self.shared.set_peer(None);
        if self.code != StatusCode::Error {
            self.code = StatusCode::Stopped;
            self.message = "Gestoppt".into();
            self.write_status();
        }
    }

    fn handle(&mut self, e: Event) {
        match e {
            Event::Send(d, to) => {
                let _ = self.socket.send_to(&d, to);
            }
            Event::Active(p) => {
                self.shared.last_hello.store(self.phone.last_hello(), Ordering::Release);
                let same = self.shared.peer.lock().unwrap().as_ref().is_some_and(|c| Arc::ptr_eq(c, &p));
                if !same {
                    self.shared.set_peer(Some(p));
                }
            }
            Event::Stopped => self.shared.set_peer(None),
            Event::Woken(_) => {
                if self.capture.is_none() {
                    if let Err(m) = self.start_capture(true) {
                        self.code = StatusCode::Error;
                        self.message = format!("Fehler: {m}");
                        self.phone.drop_peer();
                        self.phone.standby = true;
                    }
                }
            }
            Event::PairingChanged(p) => {
                self.pairing_file.set(Some(&p));
                let _ = self.pairing_file.save(&self.setup.files);
            }
        }
    }

    fn announce(&mut self) {
        let Some(p) = self.phone.pairing.clone() else { return };
        let Some(IpAddr::V4(pc)) = p.pc else { return };
        self.pairing_file.announce_counter += 1;
        let _ = self.pairing_file.save(&self.setup.files);
        let d = phone::announce(&p, self.pairing_file.announce_counter, self.setup.settings.port);
        let _ = self.socket.send_to(&d, SocketAddr::new(IpAddr::V4(pc), wire::ANNOUNCE_PORT));
    }

    fn refresh_status(&mut self, now: u64) {
        let muted = self.shared.muted.load(Ordering::Relaxed);
        let active = self.phone.active(now).cloned();
        if self.capture.is_some() {
            self.code = if active.is_some() { StatusCode::Streaming } else { StatusCode::Ready };
            self.message = match &active {
                Some(p) => {
                    let what = if muted { "Stumm".to_string() } else { self.plan_label.clone() };
                    let fast = if now < self.shared.fast_until.load(Ordering::Relaxed) { " · Schnellstart" } else { "" };
                    format!("{what}{fast} → {}", p.addr.ip())
                }
                None => "Bereit · am PC mit Handy-IP verbinden".into(),
            };
        }
        self.write_status_with(active.map(|p| p.addr));
    }

    fn write_status(&mut self) {
        self.write_status_with(None);
    }

    fn write_status_with(&mut self, peer: Option<SocketAddr>) {
        let s = &self.shared;
        let mut f = 0;
        if s.muted.load(Ordering::Relaxed) {
            f |= flag::MUTED;
        }
        if s.live.load(Ordering::Relaxed) {
            f |= flag::LIVE;
        }
        if self.phone.pairing.is_some() {
            f |= flag::PAIRED;
        }
        if now_ms() < s.fast_until.load(Ordering::Relaxed) {
            f |= flag::FAST_START;
        }
        let capturing = self.capture.is_some();
        let status = Status {
            code: self.code,
            channels: if capturing { self.capture_channels } else { 0 },
            flags: f,
            peak: if capturing { s.peak.load(Ordering::Relaxed) as u16 } else { 0 },
            left_peak: if capturing { s.left_peak.load(Ordering::Relaxed) as u16 } else { 0 },
            right_peak: if capturing { s.right_peak.load(Ordering::Relaxed) as u16 } else { 0 },
            peer: match peer {
                Some(SocketAddr::V4(a)) => a.ip().octets(),
                _ => [0; 4],
            },
            packets: s.packets.load(Ordering::Relaxed),
            pid: std::process::id(),
            message: self.message.clone(),
        };
        self.status.write(&status);
    }
}

trait StandbyAllowed {
    fn standby_allowed(&self, now: u64) -> bool;
}

impl StandbyAllowed for PhoneControl {
    /// No HELLO for a full lease (or STOP received).
    fn standby_allowed(&self, now: u64) -> bool {
        now.saturating_sub(self.last_hello()) >= LEASE_MS
    }
}
