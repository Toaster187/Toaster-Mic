//! Network side of the PC: key exchange, heartbeat, authenticated audio
//! reception into the jitter buffer, optional Always-On wake-up and pairing.
//! Platform independent; the audio output is injected as a factory.

use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use toaster_core::keyx::{self, KeyExchange, FROM_PC, FROM_PHONE};
use toaster_core::wire::{self, flags, AUDIO, HELLO, PAIR, STOP, WAKE};
use toaster_core::{now_ms, AudioPacket, ConnectionLease, ControlKey, Playout, Processor, Profile, SessionKey, Stats};

pub type FaultSink = Arc<dyn Fn(String) + Send + Sync>;
/// Starts the audio output that drains the playout. Dropping the returned
/// value stops it (may block while the driver shuts down).
pub type OutputFactory = Box<dyn FnOnce(Arc<Mutex<Playout>>, FaultSink) -> std::io::Result<Box<dyn Send>> + Send>;

#[derive(Clone)]
pub struct WakeConfig {
    pub secret: [u8; 32],
    /// First counter of a block reserved for this wake-up.
    pub counter: u64,
}

pub struct Config {
    pub host: Ipv4Addr,
    pub port: u16,
    pub target_ms: u32,
    pub gain: f32,
    pub muted: bool,
    /// Wake a paired phone from standby instead of plain offers.
    pub wake: Option<WakeConfig>,
    /// Hand this pairing secret to the phone once connected.
    pub pair: Option<([u8; 32], u64)>,
    pub studio: Option<Box<dyn Processor>>,
}

struct Shared {
    playout: Arc<Mutex<Playout>>,
    stop: AtomicBool,
    lease: ConnectionLease,
    last_audio: AtomicU64,
    channels: AtomicU8,
    flags: AtomicU8,
    fault: Mutex<Option<String>>,
    key: Mutex<Option<SessionKey>>,
    paired: AtomicBool,
    session: u64,
    first_audio_ms: AtomicU64,
    started_ms: u64,
}

pub struct Receiver {
    shared: Arc<Shared>,
    socket: UdpSocket,
    phone: SocketAddrV4,
    thread: Option<JoinHandle<()>>,
    output: Option<Box<dyn Send>>,
    pub woke: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Snapshot {
    pub stats: Stats,
    pub target_ms: u32,
    pub trimmed_ms: u32,
    pub peak: i32,
    pub silent_frames: u64,
    pub studio: bool,
}

impl Receiver {
    pub fn start(mut config: Config, output: OutputFactory) -> std::io::Result<Receiver> {
        // Not connected: NATs (e.g. the Android emulator) may answer from
        // another port. Datagrams are filtered by the phone's address and all
        // audio/control is authenticated with the session key anyway.
        let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))?;
        let phone = SocketAddrV4::new(config.host, config.port);
        let mut playout = Playout::new(config.target_ms, true, config.studio.take());
        playout.gain = config.gain;
        playout.muted = config.muted;
        let shared = Arc::new(Shared {
            playout: Arc::new(Mutex::new(playout)),
            stop: AtomicBool::new(false),
            lease: ConnectionLease::new(Duration::from_secs(3)),
            last_audio: AtomicU64::new(0),
            channels: AtomicU8::new(0),
            flags: AtomicU8::new(0),
            fault: Mutex::new(None),
            key: Mutex::new(None),
            paired: AtomicBool::new(false),
            session: toaster_core::random_u64(),
            first_audio_ms: AtomicU64::new(0),
            started_ms: now_ms(),
        });
        let fault: FaultSink = {
            let shared = shared.clone();
            Arc::new(move |m| *shared.fault.lock().unwrap() = Some(format!("Audioausgabe: {m}")))
        };
        let output = output(shared.playout.clone(), fault)?;
        let net_socket = socket.try_clone()?;
        let woke = config.wake.is_some();
        let thread = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("toaster-net".into())
                .spawn(move || network(shared, net_socket, phone, config.wake, config.pair))?
        };
        Ok(Receiver { shared, socket, phone, thread: Some(thread), output: Some(output), woke })
    }

    pub fn connected(&self) -> bool {
        self.shared.fault.lock().unwrap().is_none() && self.shared.lease.is_active()
    }

    pub fn paired(&self) -> bool {
        self.shared.paired.load(Ordering::Acquire)
    }

    pub fn channels(&self) -> u8 {
        self.shared.channels.load(Ordering::Relaxed)
    }

    /// Milliseconds from start until the first audio packet, if any.
    pub fn first_audio_after_ms(&self) -> Option<u64> {
        match self.shared.first_audio_ms.load(Ordering::Acquire) {
            0 => None,
            t => Some(t.saturating_sub(self.shared.started_ms)),
        }
    }

    pub fn status(&self) -> String {
        if let Some(f) = self.shared.fault.lock().unwrap().clone() {
            return f;
        }
        let flags_now = self.shared.flags.load(Ordering::Relaxed);
        if self.shared.last_audio.load(Ordering::Relaxed) == 0 {
            return if self.woke { "Wecke Handy …".into() } else { "Warte auf Handy …".into() };
        }
        if !self.connected() {
            return "Verbinde erneut …".into();
        }
        if flags_now & flags::MUTED != 0 {
            return "Verbunden · Handy stumm".into();
        }
        let mode = match (self.channels(), flags_now & flags::STUDIO != 0) {
            (_, true) if flags_now & flags::FAST_START != 0 => "Studio · Schnellstart",
            (_, true) => "Studio",
            (1, _) => "Mono",
            _ => "Stereo",
        };
        format!("Verbunden · {mode}")
    }

    pub fn snapshot(&self) -> Snapshot {
        let p = self.shared.playout.lock().unwrap();
        Snapshot {
            stats: p.jitter.stats(),
            target_ms: p.jitter.target_ms(),
            trimmed_ms: p.jitter.trimmed_ms(),
            peak: p.peak(),
            silent_frames: p.silent_frames,
            studio: p.studio_active(),
        }
    }

    pub fn set_gain(&self, gain: f32) {
        self.shared.playout.lock().unwrap().gain = gain;
    }

    pub fn set_muted(&self, muted: bool) {
        self.shared.playout.lock().unwrap().muted = muted;
    }

    /// Non-blocking disconnect: STOP is sent immediately, network thread and
    /// audio output are released on a background thread.
    pub fn shutdown(mut self) -> JoinHandle<()> {
        self.shared.playout.lock().unwrap().muted = true;
        self.shared.lease.clear();
        self.shared.stop.store(true, Ordering::Release);
        if let Some(key) = self.shared.key.lock().unwrap().clone() {
            let _ = self.socket.send_to(&key.control().seal(STOP, self.shared.session, 0, &[]), self.phone);
        }
        let thread = self.thread.take();
        let output = self.output.take();
        std::thread::spawn(move || {
            if let Some(t) = thread {
                let _ = t.join();
            }
            drop(output);
        })
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.shared.stop.store(true, Ordering::Release);
        }
    }
}

const WAKE_RETRY_MS: u64 = 150;
const WAKE_WINDOW_MS: u64 = 10_000;

fn network(shared: Arc<Shared>, socket: UdpSocket, phone: SocketAddrV4, wake: Option<WakeConfig>, pair: Option<([u8; 32], u64)>) {
    let exchange = KeyExchange::new();
    let session = shared.session;
    let offer = exchange.offer(FROM_PC, session);
    let wake_key = wake.as_ref().map(|w| ControlKey::new(&w.secret));
    let mut wake_counter = wake.as_ref().map_or(0, |w| w.counter);
    let started = now_ms();
    let mut next_beat = 0u64;
    let mut timeout = Duration::ZERO;
    let mut buf = vec![0u8; 2048];

    while !shared.stop.load(Ordering::Acquire) {
        let now = now_ms();
        let key = shared.key.lock().unwrap().clone();
        let waking = wake_key.is_some() && key.is_none() && now.saturating_sub(started) < WAKE_WINDOW_MS;
        if now >= next_beat {
            if waking {
                let wk = wake_key.as_ref().unwrap();
                let _ = socket.send_to(&wk.seal(WAKE, session, wake_counter, &offer), phone);
                wake_counter += 1;
                next_beat = now + WAKE_RETRY_MS;
            } else {
                if !shared.lease.is_active() {
                    let _ = socket.send_to(&offer, phone);
                }
                if let Some(k) = &key {
                    let _ = socket.send_to(&k.control().seal(HELLO, session, 0, &[]), phone);
                    if let Some((secret, counter)) = &pair {
                        if !shared.paired.load(Ordering::Acquire) && shared.lease.is_active() {
                            let masked = wire::pair_payload(k, session, *counter, secret);
                            let _ = socket.send_to(&k.control().seal(PAIR, session, *counter, &masked), phone);
                        }
                    }
                }
                next_beat = now + 1000;
            }
        }
        let wanted = Duration::from_millis(if waking { WAKE_RETRY_MS } else { 1000 });
        if wanted != timeout {
            timeout = wanted;
            let _ = socket.set_read_timeout(Some(timeout));
        }
        let n = match socket.recv_from(&mut buf) {
            Ok((n, from)) if from.ip() == std::net::IpAddr::V4(*phone.ip()) => n,
            Ok(_) => continue,
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => continue,
            Err(e) if e.kind() == ErrorKind::ConnectionReset => {
                // ICMP port unreachable: phone app not running yet.
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => {
                if !shared.stop.load(Ordering::Acquire) {
                    *shared.fault.lock().unwrap() = Some(format!("Netzwerk: {e}"));
                }
                break;
            }
        };
        let data = &mut buf[..n];
        if keyx::is_offer(data, FROM_PHONE) && keyx::offer_session(data) == session {
            if shared.lease.is_active() {
                continue;
            }
            if let Some(next) = exchange.derive(data) {
                let fresh = SessionKey::new(&next);
                let mut slot = shared.key.lock().unwrap();
                // Reset only when the phone negotiated a different key (restart).
                if slot.is_none() || !same_key(slot.as_ref().unwrap(), &fresh, session) {
                    shared.playout.lock().unwrap().reset();
                    shared.paired.store(false, Ordering::Release);
                    *slot = Some(fresh.clone());
                }
                drop(slot);
                let _ = socket.send_to(&fresh.control().seal(HELLO, session, 0, &[]), phone);
            }
            continue;
        }
        let Some(key) = key.or_else(|| shared.key.lock().unwrap().clone()) else { continue };
        let Some(h) = wire::parse_header(data) else { continue };
        if h.session != session {
            continue;
        }
        if h.kind == AUDIO {
            if key.open_audio(data).is_none() {
                continue;
            }
            let mut samples = vec![0i16; (n - wire::HEADER - wire::TAG) / 2];
            wire::read_pcm(&data[wire::HEADER..n - wire::TAG], &mut samples);
            let now = now_ms();
            shared.lease.observe_at(now);
            if shared.last_audio.swap(now, Ordering::Relaxed) == 0 {
                shared.first_audio_ms.store(now, Ordering::Release);
            }
            shared.channels.store(h.channels, Ordering::Relaxed);
            shared.flags.store(h.flags, Ordering::Relaxed);
            let packet = AudioPacket { sequence: h.sequence, channels: h.channels, flags: h.flags, samples };
            let mut p = shared.playout.lock().unwrap();
            p.jitter.set_profile_floor_ms(Profile::from_flags(h.flags).min_reserve_ms());
            p.jitter.push(packet);
        } else if h.kind == PAIR && key.control().open(data).is_some() {
            shared.paired.store(true, Ordering::Release);
        }
    }
}

/// Two `SessionKey`s are equal when they authenticate the same HELLO.
fn same_key(a: &SessionKey, b: &SessionKey, session: u64) -> bool {
    a.control().seal(HELLO, session, 0, &[]) == b.control().seal(HELLO, session, 0, &[])
}
