//! The studio chain. Runs on the PC audio thread in 10-ms blocks:
//!
//! high-pass -> channel check -> GCC-PHAT alignment -> SNR-weighted sum
//! -> AGC pre-gain -> RNNoise -> (fast-start crossfade) -> user gain
//! -> compressor -> look-ahead limiter (-1 dBFS)
//!
//! All buffers are allocated when the channel count is configured; the
//! per-packet path does not allocate.

use nnnoiseless::DenoiseState;
use toaster_core::wire::{flags, RATE};
use toaster_core::Processor;

use crate::align::{Gcc, MAX_LAG};
use crate::filters::{coeff, DelayLine, HighPass};
use crate::NoiseLevel;

pub const BLOCK: usize = DenoiseState::FRAME_SIZE; // 480 frames = 10 ms
/// Read position of the alignment delay lines (>= MAX_LAG + half sinc).
const BASE: f32 = (MAX_LAG + 8) as f32;
/// RNNoise output lags its input by one frame (overlap-add synthesis).
pub const NS_DELAY: usize = BLOCK;
const LOOKAHEAD: usize = 72; // 1.5 ms
/// -1 dBFS in PCM16 units.
pub const CEILING: f32 = 32768.0 * 0.891_250_9;
/// Speech level the AGC aims for: about -18 dBFS RMS.
const TARGET_RMS: f32 = 32768.0 * 0.125_9;
const MAX_AGC: f32 = 100.0; // +40 dB, raw microphones are very quiet
const HISTORY: usize = 2 * BLOCK;

struct Channel {
    hp: HighPass,
    line: DelayLine,
    block: Vec<f32>,
    history: Vec<f32>,
    energy: f32,
    noise: f32,
    signal: f32,
    weight: f32,
    delay: f32,
    target: f32,
    estimates: u32,
    outliers: u32,
    active: bool,
}

impl Channel {
    fn new() -> Self {
        Channel {
            hp: HighPass::new(80.0, RATE as f32),
            line: DelayLine::new(256),
            block: vec![0.0; BLOCK],
            history: vec![0.0; HISTORY],
            energy: 0.0,
            noise: -1.0,
            signal: 0.0,
            weight: 0.0,
            delay: 0.0,
            target: 0.0,
            estimates: 0,
            outliers: 0,
            active: true,
        }
    }
}

pub struct Studio {
    noise_level: NoiseLevel,
    ch: Vec<Channel>,
    fill: usize,
    reference: usize,
    gcc: Gcc,
    ns: Box<DenoiseState<'static>>,
    ns_in: Vec<f32>,
    ns_out: Vec<f32>,
    combined: Vec<f32>,
    fast: Vec<f32>,
    dry_line: DelayLine,
    fast_line: DelayLine,
    vad: f32,
    speech_blocks: u32,
    speech_rms: f32,
    agc: f32,
    agc_target: f32,
    fade: f32,
    comp_env: f32,
    lim_line: DelayLine,
    lim_peaks: Vec<f32>,
    lim_pos: usize,
    lim_gain: f32,
    out: Vec<f32>,
    out_head: usize,
    out_len: usize,
    user_gain: f32,
    pub frozen: bool,
    /// Bypass noise suppression and dynamics (linear path, for measurements).
    pub linear: bool,
}

impl Studio {
    pub fn new(noise_level: NoiseLevel) -> Self {
        let mut s = Studio {
            noise_level,
            ch: Vec::new(),
            fill: 0,
            reference: 0,
            gcc: Gcc::new(),
            ns: DenoiseState::new(),
            ns_in: vec![0.0; BLOCK],
            ns_out: vec![0.0; BLOCK],
            combined: vec![0.0; BLOCK],
            fast: vec![0.0; BLOCK],
            dry_line: DelayLine::new(NS_DELAY + 8),
            fast_line: DelayLine::new(NS_DELAY + 8),
            vad: 0.0,
            speech_blocks: 0,
            speech_rms: 0.0,
            agc: 8.0,
            agc_target: 8.0,
            fade: 0.0,
            comp_env: 0.0,
            lim_line: DelayLine::new(LOOKAHEAD + 8),
            lim_peaks: vec![0.0; LOOKAHEAD + 1],
            lim_pos: 0,
            lim_gain: 1.0,
            out: vec![0.0; 4096],
            out_head: 0,
            out_len: 0,
            user_gain: 1.0,
            frozen: false,
            linear: false,
        };
        s.configure(3);
        let warm = DelayLine::new(64);
        let _ = warm.read(20.5);
        s
    }

    pub fn noise(&self) -> NoiseLevel {
        self.noise_level
    }

    fn configure(&mut self, channels: usize) {
        self.ch = (0..channels.max(1)).map(|_| Channel::new()).collect();
        self.reset_state();
    }

    fn reset_state(&mut self) {
        for c in &mut self.ch {
            *c = Channel::new();
        }
        self.fill = 0;
        self.reference = 0;
        self.ns = DenoiseState::new();
        self.dry_line.reset();
        self.fast_line.reset();
        self.lim_line.reset();
        self.lim_peaks.fill(0.0);
        self.vad = 0.0;
        self.speech_blocks = 0;
        self.speech_rms = 0.0;
        self.agc = 8.0;
        self.agc_target = 8.0;
        self.fade = 0.0;
        self.comp_env = 0.0;
        self.lim_gain = 1.0;
        // One block of silence covers the time until the first block is complete.
        self.out.fill(0.0);
        self.out_head = 0;
        self.out_len = BLOCK;
    }

    /// Constant processing latency in frames.
    pub fn latency_frames(&self) -> usize {
        BLOCK + NS_DELAY + BASE as usize + LOOKAHEAD
    }
    pub fn delays(&self) -> Vec<f32> {
        self.ch.iter().map(|c| c.target).collect()
    }
    pub fn reference(&self) -> usize {
        self.reference
    }
    pub fn weights(&self) -> Vec<f32> {
        self.ch.iter().map(|c| c.weight).collect()
    }
    pub fn agc_gain(&self) -> f32 {
        self.agc
    }
    pub fn fade(&self) -> f32 {
        self.fade
    }

    fn push_out(&mut self, v: f32) {
        let n = self.out.len();
        let idx = (self.out_head + self.out_len) % n;
        self.out[idx] = v;
        if self.out_len < n {
            self.out_len += 1;
        } else {
            self.out_head = (self.out_head + 1) % n;
        }
    }

    fn pop_out(&mut self) -> f32 {
        if self.out_len == 0 {
            return 0.0;
        }
        let v = self.out[self.out_head];
        self.out_head = (self.out_head + 1) % self.out.len();
        self.out_len -= 1;
        v
    }

    fn run_block(&mut self, fast_start: bool) {
        let n = self.ch.len();
        // 1. Levels, noise floors, dead or duplicated channels.
        for c in &mut self.ch {
            c.energy = c.block.iter().map(|x| x * x).sum::<f32>() / BLOCK as f32;
            if !self.frozen {
                c.noise = if c.noise < 0.0 {
                    c.energy
                } else if c.energy < c.noise {
                    0.7 * c.noise + 0.3 * c.energy
                } else {
                    c.noise * 1.02
                };
            }
            c.history.copy_within(BLOCK.., 0);
            c.history[HISTORY - BLOCK..].copy_from_slice(&c.block);
        }
        if !self.frozen {
            for i in 0..n {
                let mut active = self.ch[i].energy > 1e-4;
                for j in 0..i {
                    if !self.ch[j].active {
                        continue;
                    }
                    let (a, b) = (&self.ch[i].block, &self.ch[j].block);
                    let (mut ab, mut aa, mut bb) = (0.0f32, 0.0f32, 0.0f32);
                    for k in 0..BLOCK {
                        ab += a[k] * b[k];
                        aa += a[k] * a[k];
                        bb += b[k] * b[k];
                    }
                    if aa > 0.0 && bb > 0.0 && ab / (aa * bb).sqrt() > 0.9995 {
                        active = false; // exact copy of another channel
                    }
                }
                self.ch[i].active = active;
            }
        }
        // 2. Speech detection (energy, plus RNNoise VAD of the previous block).
        let max_snr = self.ch.iter().filter(|c| c.active).map(|c| c.energy / c.noise.max(1e-3)).fold(0.0f32, f32::max);
        let speech = max_snr > 4.0 || self.vad > 0.6;
        if speech && !self.frozen {
            for c in &mut self.ch {
                let s = (c.energy - c.noise).max(0.0).sqrt();
                c.signal = if c.signal == 0.0 { s } else { 0.9 * c.signal + 0.1 * s };
            }
            // Reference: best SNR, with hysteresis.
            let snr = |c: &Channel| if c.active { c.signal * c.signal / c.noise.max(1e-3) } else { 0.0 };
            let best = (0..n).max_by(|&a, &b| snr(&self.ch[a]).total_cmp(&snr(&self.ch[b]))).unwrap_or(0);
            if snr(&self.ch[best]) > 1.5 * snr(&self.ch[self.reference]) || !self.ch[self.reference].active {
                if best != self.reference {
                    let shift = self.ch[best].target;
                    for c in &mut self.ch {
                        c.target -= shift;
                        c.delay -= shift;
                    }
                    self.reference = best;
                }
            }
            // 3. Time differences against the reference microphone.
            let r = self.reference;
            for i in 0..n {
                if i == r || !self.ch[i].active {
                    continue;
                }
                let (d, confidence) = self.gcc.delay(&self.ch[r].history, &self.ch[i].history);
                if confidence < 3.0 {
                    continue;
                }
                let c = &mut self.ch[i];
                if c.estimates == 0 || c.outliers >= 3 {
                    c.target = d;
                    c.outliers = 0;
                } else if (d - c.target).abs() < 3.0 {
                    c.target = 0.85 * c.target + 0.15 * d;
                    c.outliers = 0;
                } else {
                    c.outliers += 1;
                }
                c.estimates += 1;
            }
            self.ch[r].target = 0.0;
            self.speech_blocks += 1;
        }
        // 4. Maximum-ratio weights, normalised to the reference speech level.
        let r = self.reference;
        let ref_signal = self.ch[r].signal;
        let mut raw = [0.0f32; 8];
        let mut norm = 0.0;
        for (i, c) in self.ch.iter().enumerate() {
            if !c.active {
                continue;
            }
            raw[i] = if ref_signal > 0.0 { c.signal / c.noise.max(1e-3) } else { 1.0 / c.noise.max(1e-3).sqrt() };
            norm += raw[i] * if ref_signal > 0.0 { c.signal } else { c.noise.max(1e-3).sqrt() };
        }
        let ref_level = if ref_signal > 0.0 { ref_signal } else { self.ch[r].noise.max(1e-3).sqrt() };
        let scale = if norm > 0.0 { ref_level / norm } else { 0.0 };
        let old: [f32; 8] = std::array::from_fn(|i| self.ch.get(i).map_or(0.0, |c| c.weight));
        if !self.frozen {
            for (i, c) in self.ch.iter_mut().enumerate() {
                let w = raw[i] * scale;
                c.weight = if c.weight == 0.0 { w } else { 0.8 * c.weight + 0.2 * w };
            }
        }
        // 5. Align (fractional delays, slewed) and combine.
        for k in 0..BLOCK {
            let t = k as f32 / BLOCK as f32;
            let mut sum = 0.0;
            let mut fast = 0.0;
            for (i, c) in self.ch.iter_mut().enumerate() {
                c.line.push(c.block[k]);
                c.delay += (c.target - c.delay).clamp(-0.001, 0.001);
                let y = c.line.read(BASE - c.delay.clamp(-(MAX_LAG as f32), MAX_LAG as f32));
                sum += y * (old[i] + (c.weight - old[i]) * t);
                if i == r {
                    fast = y;
                }
            }
            self.combined[k] = sum;
            self.fast[k] = fast;
        }
        if self.linear {
            for k in 0..BLOCK {
                let v = self.combined[k];
                self.push_out(v);
            }
            return;
        }
        // 6. AGC pre-gain so that RNNoise sees normal speech levels.
        let agc_from = self.agc;
        for k in 0..BLOCK {
            let g = agc_from + (self.agc_target - agc_from) * (k as f32 / BLOCK as f32);
            self.ns_in[k] = (self.combined[k] * g).clamp(-32767.0, 32767.0);
            self.fast[k] = (self.fast[k] * g).clamp(-32767.0, 32767.0);
        }
        self.agc = self.agc_target;
        let vad = match self.noise_level {
            NoiseLevel::Off => {
                self.ns_out.copy_from_slice(&self.ns_in);
                if speech { 1.0 } else { 0.0 }
            }
            _ => self.ns.process_frame(&mut self.ns_out, &self.ns_in),
        };
        self.vad = vad;
        let dry_mix = match self.noise_level {
            NoiseLevel::Medium => 0.12,
            _ => 0.0,
        };
        let ns_delay = if self.noise_level == NoiseLevel::Off { 0 } else { NS_DELAY };
        // 7. Fast-start crossfade (time-aligned: both paths share every delay).
        let fade_to = if fast_start { 0.0 } else { 1.0 };
        let user_gain = self.user_gain;
        let release = coeff(0.12, RATE as f32);
        let attack = coeff(0.005, RATE as f32);
        let lim_release = coeff(0.05, RATE as f32);
        let threshold = 32768.0 * 0.25;
        for k in 0..BLOCK {
            self.dry_line.push(self.ns_in[k]);
            self.fast_line.push(self.fast[k]);
            let dry = self.dry_line.at(ns_delay);
            let full = self.ns_out[k] * (1.0 - dry_mix) + dry * dry_mix;
            let fast = self.fast_line.at(ns_delay);
            self.fade += (fade_to - self.fade).clamp(-1.0 / BLOCK as f32, 1.0 / BLOCK as f32);
            let mut v = (self.fade * full + (1.0 - self.fade) * fast) * user_gain;
            // 8. Compressor 3:1 above -12 dBFS.
            let a = v.abs();
            let c = if a > self.comp_env { attack } else { release };
            self.comp_env = a + c * (self.comp_env - a);
            if self.comp_env > threshold {
                v *= (threshold / self.comp_env).powf(1.0 - 1.0 / 3.0);
            }
            // 9. Look-ahead limiter: the window covers every sample until output.
            self.lim_line.push(v);
            self.lim_peaks[self.lim_pos] = v.abs();
            self.lim_pos = (self.lim_pos + 1) % self.lim_peaks.len();
            let peak = self.lim_peaks.iter().copied().fold(0.0f32, f32::max);
            let wanted = if peak > CEILING { CEILING / peak } else { 1.0 };
            self.lim_gain = if wanted < self.lim_gain { wanted } else { wanted + lim_release * (self.lim_gain - wanted) };
            let y = (self.lim_line.at(LOOKAHEAD) * self.lim_gain).clamp(-CEILING, CEILING);
            self.push_out(y);
        }
        // 10. AGC update from speech blocks only (holds during pauses).
        if !self.frozen && (vad > 0.6 || (self.noise_level == NoiseLevel::Off && speech)) {
            let rms = (self.combined.iter().map(|x| x * x).sum::<f32>() / BLOCK as f32).sqrt();
            let alpha = if self.speech_blocks < 50 { 0.8 } else { 0.985 };
            self.speech_rms = if self.speech_rms == 0.0 { rms } else { alpha * self.speech_rms + (1.0 - alpha) * rms };
            if self.speech_rms > 0.0 {
                let desired = (TARGET_RMS / self.speech_rms).clamp(1.0, MAX_AGC);
                // Fast to reduce, slow to raise.
                let k = if desired < self.agc_target { 0.3 } else { 0.03 };
                self.agc_target += (desired - self.agc_target) * k;
            }
        }
    }
}

impl Processor for Studio {
    fn process(&mut self, input: &[i16], channels: usize, packet_flags: u8, gain: f32, out: &mut [f32]) {
        if channels != self.ch.len() {
            self.configure(channels);
        }
        self.user_gain = gain;
        let fast = packet_flags & flags::FAST_START != 0;
        let frames = input.len() / channels;
        for f in 0..frames {
            for (c, ch) in self.ch.iter_mut().enumerate() {
                ch.block[self.fill] = ch.hp.process(input[f * channels + c] as f32);
            }
            self.fill += 1;
            if self.fill == BLOCK {
                self.fill = 0;
                self.run_block(fast);
            }
        }
        for f in 0..frames.min(out.len() / 2) {
            let v = self.pop_out();
            out[f * 2] = v;
            out[f * 2 + 1] = v;
        }
    }

    fn reset(&mut self) {
        self.reset_state();
    }

    fn quiet_gain(&self) -> f32 {
        self.agc
    }
}
