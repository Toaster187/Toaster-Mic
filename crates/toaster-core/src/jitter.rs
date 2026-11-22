//! Bounded-latency jitter buffer for fixed-size audio packets.
//!
//! Reorders inside a window, conceals missing packets, discards old audio
//! when the consumer falls behind, adapts its reserve to underruns and to
//! the audio device period, and gives reserve back in verified speech pauses.
//! Port of the 0.4 `JitterBuffer.cs`, generalised from fixed 5-ms packets to
//! any packet size (5 ms for mono/stereo, 3.33 ms for 3-4 channels).

use std::collections::HashMap;

use crate::wire::RATE;

#[derive(Clone, Debug, PartialEq)]
pub struct AudioPacket {
    pub sequence: u32,
    pub channels: u8,
    pub flags: u8,
    /// Interleaved PCM16, `frames * channels` samples.
    pub samples: Vec<i16>,
}

impl AudioPacket {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }
}

#[derive(Debug)]
pub enum Next {
    /// Not playing yet (startup or refill): output silence.
    Idle,
    /// Play this packet. `fade_in` after a gap or a trim.
    Packet { packet: AudioPacket, fade_in: bool },
    /// Packet missing: conceal by fading out.
    Conceal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub received: u64,
    pub lost: u64,
    pub late: u64,
    pub resyncs: u64,
    pub buffered_ms: u32,
}

const MAX_RESERVE_FRAMES: usize = RATE as usize * 60 / 1000;

pub struct JitterBuffer {
    configured_ms: u32,
    adaptive: bool,
    packets: HashMap<u32, AudioPacket>,
    next: u32,
    initialized: bool,
    playing: bool,
    silent_run: usize,
    lost: u64,
    late: u64,
    received: u64,
    resyncs: u64,
    trimmed: u64,
    effective: usize,
    profile_floor_ms: u32,
    device_floor: usize,
    observed_callback: usize,
    callback_frames: usize,
    quiet_packets: usize,
    stable_packets: usize,
    recovery_cooldown: usize,
    recovering: bool,
    packet_frames: usize,
    /// Gain the output applies to raw samples; makes pause detection conservative.
    pub quiet_gain: f32,
}

impl JitterBuffer {
    pub fn new(target_ms: u32, adaptive: bool) -> Self {
        let mut jb = JitterBuffer {
            configured_ms: target_ms,
            adaptive,
            packets: HashMap::with_capacity(64),
            next: 0,
            initialized: false,
            playing: false,
            silent_run: 0,
            lost: 0,
            late: 0,
            received: 0,
            resyncs: 0,
            trimmed: 0,
            effective: 1,
            profile_floor_ms: 0,
            device_floor: 1,
            observed_callback: 0,
            callback_frames: 0,
            quiet_packets: 0,
            stable_packets: 0,
            recovery_cooldown: 0,
            recovering: false,
            packet_frames: 240,
            quiet_gain: 1.0,
        };
        jb.effective = jb.configured_packets();
        jb
    }

    fn max_packets(&self) -> usize {
        (MAX_RESERVE_FRAMES / self.packet_frames).max(1)
    }
    fn packets_per_second(&self) -> usize {
        RATE as usize / self.packet_frames
    }
    fn ms_to_packets(&self, ms: u32) -> usize {
        let frames = ms as usize * RATE as usize / 1000;
        frames.div_ceil(self.packet_frames)
    }
    fn configured_packets(&self) -> usize {
        self.ms_to_packets(self.configured_ms.max(self.profile_floor_ms)).clamp(1, self.max_packets())
    }
    fn packets_to_ms(&self, packets: usize) -> u32 {
        ((packets * self.packet_frames * 1000 + RATE as usize / 2) / RATE as usize) as u32
    }

    pub fn target_ms(&self) -> u32 {
        self.packets_to_ms(self.effective)
    }
    pub fn trimmed_ms(&self) -> u32 {
        self.packets_to_ms(self.trimmed as usize)
    }
    pub fn trimmed_packets(&self) -> u64 {
        self.trimmed
    }
    pub fn packet_frames(&self) -> usize {
        self.packet_frames
    }
    pub fn stats(&self) -> Stats {
        Stats {
            received: self.received,
            lost: self.lost,
            late: self.late,
            resyncs: self.resyncs,
            buffered_ms: self.packets_to_ms(self.packets.len()),
        }
    }

    /// Minimum reserve demanded by the sender's latency profile.
    pub fn set_profile_floor_ms(&mut self, ms: u32) {
        if ms != self.profile_floor_ms {
            self.profile_floor_ms = ms;
            self.effective = self.effective.max(self.configured_packets());
        }
    }

    pub fn reset(&mut self) {
        self.packets.clear();
        self.initialized = false;
        self.playing = false;
        self.recovering = false;
        self.silent_run = 0;
        self.quiet_packets = 0;
        self.stable_packets = 0;
        self.recovery_cooldown = 0;
        self.device_floor = 1;
        self.observed_callback = 0;
        self.callback_frames = 0;
        self.profile_floor_ms = 0;
        self.effective = self.configured_packets();
    }

    pub fn push(&mut self, packet: AudioPacket) {
        let frames = packet.frames();
        if frames == 0 {
            return;
        }
        if frames != self.packet_frames {
            // New stream format (channel count changed): start over.
            self.packet_frames = frames;
            self.reset();
        }
        if !self.initialized {
            self.next = packet.sequence;
            self.initialized = true;
        }
        let distance = packet.sequence.wrapping_sub(self.next) as i32;
        if distance < 0 {
            self.late += 1;
            return;
        }
        let window = self.max_packets().max(self.effective + self.max_packets() / 2);
        if distance as usize > window {
            self.packets.clear();
            self.next = packet.sequence;
            self.playing = false;
            self.quiet_packets = 0;
            self.stable_packets = 0;
            self.resyncs += 1;
        }
        if let std::collections::hash_map::Entry::Vacant(e) = self.packets.entry(packet.sequence) {
            e.insert(packet);
            self.received += 1;
        }
    }

    /// Called once per device callback with the number of frames requested.
    /// A large startup callback must not permanently set the latency floor:
    /// the largest device period of the current one-second window is kept.
    pub fn observe_callback(&mut self, frames: usize) {
        if !self.adaptive || frames == 0 {
            return;
        }
        let requested = (frames.div_ceil(self.packet_frames) + 1).clamp(1, self.max_packets());
        self.observed_callback = self.observed_callback.max(requested);
        self.device_floor = self.device_floor.max(requested);
        self.callback_frames += frames;
        if self.callback_frames >= RATE as usize {
            self.device_floor = self.observed_callback;
            self.observed_callback = 0;
            self.callback_frames = 0;
        }
        self.effective = self.effective.max(self.device_floor);
    }

    pub fn next(&mut self) -> Next {
        if !self.playing {
            if self.packets.len() < self.effective {
                return Next::Idle;
            }
            self.playing = true;
            self.silent_run = 0;
        }
        if self.adaptive {
            self.recover_during_quiet();
        }
        if let Some(packet) = self.packets.remove(&self.next) {
            self.next = self.next.wrapping_add(1);
            self.silent_run = 0;
            let pps = self.packets_per_second();
            self.stable_packets = (self.stable_packets + 1).min(2 * pps);
            self.quiet_packets = if self.is_quiet(&packet) { (self.quiet_packets + 1).min(pps) } else { 0 };
            self.recovery_cooldown = self.recovery_cooldown.saturating_sub(1);
            let fade_in = self.recovering;
            self.recovering = false;
            return Next::Packet { packet, fade_in };
        }
        self.lost += 1;
        self.quiet_packets = 0;
        self.stable_packets = 0;
        if self.adaptive {
            // Fade out a missing packet; do not chase the sender forever by
            // advancing its sequence while the entire receive queue is empty.
            self.recovering = true;
            self.effective = (self.effective + 1).min(self.max_packets());
            if self.packets.is_empty() {
                self.playing = false;
                return Next::Conceal;
            }
        }
        self.next = self.next.wrapping_add(1);
        self.silent_run += 1;
        if self.silent_run >= (RATE as usize / 20).div_ceil(self.packet_frames) {
            self.playing = false;
            self.initialized = false;
            self.packets.clear();
        }
        if self.adaptive {
            Next::Conceal
        } else {
            Next::Idle
        }
    }

    fn recover_during_quiet(&mut self) {
        // One second of real quiet audio, two seconds without an underrun.
        // Never mistake missing packets or the output mute for a speech pause.
        let pps = self.packets_per_second();
        if self.quiet_packets < pps || self.stable_packets < 2 * pps || self.recovery_cooldown > 0 {
            return;
        }
        match self.packets.get(&self.next) {
            Some(p) if self.is_quiet(p) => {}
            _ => return,
        }
        let floor = self.configured_packets().max(self.device_floor);
        let step = (RATE as usize / 100).div_ceil(self.packet_frames); // 10 ms
        self.effective = floor.max(self.effective.saturating_sub(step));
        // Only discard a contiguous, verified quiet prefix. Keep queued speech,
        // sequence numbers, the session key and the connection intact.
        let max_removed = (RATE as usize / 25) / self.packet_frames; // 40 ms
        let mut removed = 0;
        while self.packets.len() > self.effective && removed < max_removed {
            match self.packets.get(&self.next) {
                Some(p) if self.is_quiet(p) => {}
                _ => break,
            }
            self.packets.remove(&self.next);
            self.next = self.next.wrapping_add(1);
            self.trimmed += 1;
            removed += 1;
        }
        if removed > 0 {
            self.recovering = true;
        }
        self.recovery_cooldown = pps;
    }

    fn is_quiet(&self, packet: &AudioPacket) -> bool {
        let gain = if self.quiet_gain.is_finite() { self.quiet_gain.clamp(1.0, 1000.0) as f64 } else { 1.0 };
        let mut energy = 0.0f64;
        for &s in &packet.samples {
            let v = s as f64 * gain;
            if v.abs() > 300.0 {
                return false;
            }
            energy += v * v;
        }
        !packet.samples.is_empty() && energy <= packet.samples.len() as f64 * 10_000.0
    }
}
