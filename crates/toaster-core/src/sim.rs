//! Simulated phone for tests and the probe tool: real UDP, real protocol,
//! synthetic multi-microphone audio. Uses the same [`PhoneControl`] as the
//! Android engine.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::lease::now_ms;
use crate::phone::{Event, Pairing, Peer, PhoneControl};
use crate::wire::{self, flags, Profile};

#[derive(Clone)]
pub struct SimConfig {
    pub bind: Ipv4Addr,
    /// 0 picks a free port.
    pub port: u16,
    pub channels: u8,
    pub studio: bool,
    pub profile: Profile,
    pub pairing: Option<Pairing>,
    /// Start in standby: only paired WAKE is accepted.
    pub standby: bool,
    /// Return to standby after the PC stops (Always-On phone).
    pub always_on: bool,
    pub fast_start_ms: u64,
    pub tone_hz: f32,
    /// Peak amplitude of the clean signal in PCM16 units.
    pub level: f32,
    pub noise: f32,
}

impl Default for SimConfig {
    fn default() -> Self {
        SimConfig {
            bind: Ipv4Addr::LOCALHOST,
            port: 0,
            channels: 1,
            studio: false,
            profile: Profile::LowLatency,
            pairing: None,
            standby: false,
            always_on: false,
            fast_start_ms: 10_000,
            tone_hz: 220.0,
            level: 8000.0,
            noise: 300.0,
        }
    }
}

#[derive(Default)]
pub struct SimState {
    pub hellos: AtomicU32,
    pub woken: AtomicU32,
    pub stopped: AtomicU32,
    pub packets: AtomicU64,
    pub active: AtomicBool,
    pub standby: AtomicBool,
    pub pairing: Mutex<Option<Pairing>>,
    pub woken_at_ms: AtomicU64,
    pub first_packet_after_wake_ms: AtomicU64,
}

pub struct SimPhone {
    pub addr: SocketAddr,
    pub state: Arc<SimState>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for SimPhone {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Speech-like source: a glottal-like harmonic series (1/k up to ~4 kHz)
/// plus a breathy broadband part, under a syllable envelope.
pub struct Voice {
    t: u64,
    hz: f32,
    breath: Noise,
}

impl Voice {
    pub fn new(hz: f32) -> Self {
        Voice { t: 0, hz, breath: Noise::new(0x51_7CC1_B727_220A) }
    }
    pub fn next(&mut self) -> f32 {
        let t = self.t as f32 / wire::RATE as f32;
        self.t += 1;
        let w = std::f32::consts::TAU * self.hz * t;
        let env = 0.5 + 0.5 * (std::f32::consts::TAU * 3.0 * t).sin();
        let harmonics = (4000.0 / self.hz) as usize;
        let mut v = 0.0;
        for k in 1..=harmonics.max(1) {
            v += (k as f32 * w).sin() / k as f32;
        }
        env * (0.45 * v + 0.25 * self.breath.next())
    }
}

/// Deterministic white noise in [-1, 1].
pub struct Noise(u64);
impl Noise {
    pub fn new(seed: u64) -> Self {
        Noise(seed | 1)
    }
    pub fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    }
}

/// Per-microphone delays (samples) and gains of the simulated phone.
pub const MIC_DELAYS: [usize; 4] = [0, 7, 15, 3];
pub const MIC_GAINS: [f32; 4] = [1.0, 0.8, 0.45, 0.9];

/// Generates interleaved multi-microphone frames of one voice source.
pub struct MicArray {
    voice: Voice,
    history: Vec<f32>,
    pos: usize,
    noise: Vec<Noise>,
    pub level: f32,
    pub noise_level: f32,
    pub delays: [usize; 4],
    pub gains: [f32; 4],
    /// Switch the voice / the noise off (tests measure them separately).
    pub voice_on: bool,
    pub noise_on: bool,
}

impl MicArray {
    pub fn new(channels: usize, hz: f32, level: f32, noise_level: f32) -> Self {
        MicArray {
            voice: Voice::new(hz),
            history: vec![0.0; 64],
            pos: 0,
            noise: (0..channels).map(|c| Noise::new(0x9E37_79B9 + c as u64 * 7919)).collect(),
            level,
            noise_level,
            delays: MIC_DELAYS,
            gains: MIC_GAINS,
            voice_on: true,
            noise_on: true,
        }
    }
    pub fn fill(&mut self, out: &mut [i16]) {
        let ch = self.noise.len();
        for frame in out.chunks_exact_mut(ch) {
            self.pos = (self.pos + 1) % self.history.len();
            self.history[self.pos] = self.voice.next();
            for (c, s) in frame.iter_mut().enumerate() {
                let idx = (self.pos + self.history.len() - self.delays[c % 4]) % self.history.len();
                let voice = if self.voice_on { self.history[idx] * self.gains[c % 4] * self.level } else { 0.0 };
                let noise = self.noise[c].next() * self.noise_level;
                let v = voice + if self.noise_on { noise } else { 0.0 };
                *s = v.clamp(-32768.0, 32767.0) as i16;
            }
        }
    }
}

impl SimPhone {
    pub fn start(config: SimConfig) -> std::io::Result<SimPhone> {
        let socket = UdpSocket::bind((config.bind, config.port))?;
        socket.set_read_timeout(Some(Duration::from_millis(1)))?;
        let addr = socket.local_addr()?;
        let state = Arc::new(SimState::default());
        *state.pairing.lock().unwrap() = config.pairing.clone();
        state.standby.store(config.standby, Ordering::Release);
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (state, stop) = (state.clone(), stop.clone());
            std::thread::Builder::new().name("sim-phone".into()).spawn(move || run(socket, config, state, stop))?
        };
        Ok(SimPhone { addr, state, stop, thread: Some(thread) })
    }
}

fn run(socket: UdpSocket, config: SimConfig, state: Arc<SimState>, stop: Arc<AtomicBool>) {
    let channels = config.channels.clamp(1, wire::MAX_CHANNELS);
    let frames = wire::frames_for(channels) as usize;
    let packet_ms = frames as f64 * 1000.0 / wire::RATE as f64;
    let mut control = PhoneControl::new(config.pairing.clone());
    control.standby = config.standby;
    let mut mics = MicArray::new(channels as usize, config.tone_hz, config.level, config.noise);
    let mut samples = vec![0i16; frames * channels as usize];
    let mut buf = vec![0u8; wire::MAX_DATAGRAM];
    let mut rx = [0u8; 2048];
    let mut events = Vec::new();
    let mut sequence = 0u32;
    let mut fast_until = 0u64;
    let mut clock_start = now_ms();
    let mut sent_packets = 0u64;
    let mut was_active = false;

    while !stop.load(Ordering::Acquire) {
        if let Ok((n, from)) = socket.recv_from(&mut rx) {
            events.clear();
            control.on_datagram(&rx[..n], from, now_ms(), &mut events);
            for e in events.drain(..) {
                match e {
                    Event::Send(d, to) => {
                        let _ = socket.send_to(&d, to);
                    }
                    Event::Active(_) => {
                        state.hellos.fetch_add(1, Ordering::Relaxed);
                    }
                    Event::Stopped => {
                        state.stopped.fetch_add(1, Ordering::Relaxed);
                    }
                    Event::Woken(_) => {
                        state.woken.fetch_add(1, Ordering::Relaxed);
                        state.woken_at_ms.store(now_ms(), Ordering::Release);
                        state.first_packet_after_wake_ms.store(0, Ordering::Release);
                        fast_until = now_ms() + config.fast_start_ms;
                    }
                    Event::PairingChanged(p) => {
                        *state.pairing.lock().unwrap() = Some(p);
                    }
                }
            }
        }
        let now = now_ms();
        let active: Option<Arc<Peer>> = control.active(now).cloned();
        state.active.store(active.is_some(), Ordering::Relaxed);
        if active.is_none() {
            if was_active && config.always_on {
                control.drop_peer();
                control.standby = true;
                state.standby.store(true, Ordering::Release);
            }
            was_active = false;
            clock_start = now;
            sent_packets = 0;
            continue;
        }
        if !was_active {
            was_active = true;
            state.standby.store(false, Ordering::Release);
        }
        let peer = active.unwrap();
        // Catch up on all packets due since the stream clock started.
        let due = ((now - clock_start) as f64 / packet_ms) as u64 + 1;
        while sent_packets < due {
            mics.fill(&mut samples);
            wire::write_pcm(&samples, &mut buf[wire::HEADER..]);
            let mut f = config.profile.bits();
            if config.studio {
                f |= flags::STUDIO;
            }
            if now < fast_until {
                f |= flags::FAST_START;
            }
            let len = peer.key.seal_audio(&mut buf, channels, f, peer.session, sequence);
            sequence = sequence.wrapping_add(1);
            let _ = socket.send_to(&buf[..len], peer.addr);
            sent_packets += 1;
            state.packets.fetch_add(1, Ordering::Relaxed);
            if state.first_packet_after_wake_ms.load(Ordering::Acquire) == 0 {
                let woke = state.woken_at_ms.load(Ordering::Acquire);
                if woke != 0 {
                    state.first_packet_after_wake_ms.store((now - woke).max(1), Ordering::Release);
                }
            }
        }
    }
}
