//! Turns jitter-buffered packets into a continuous stereo PCM16 stream for the
//! audio device. Signal processing is pluggable ([`Processor`]): plain
//! pass-through for mono/stereo, the studio chain for multi-microphone audio.

use crate::jitter::{JitterBuffer, Next};
use crate::wire::flags;

/// Converts one packet of interleaved input into stereo output samples
/// (f32 in PCM16 scale, `frames * 2` values). Must not allocate.
pub trait Processor: Send {
    fn process(&mut self, input: &[i16], channels: usize, flags: u8, gain: f32, out: &mut [f32]);
    /// Produce output for a missing packet. Processors with internal latency
    /// flush their tail here; return `false` to let the playout fade out.
    fn conceal(&mut self, _out: &mut [f32]) -> bool {
        false
    }
    fn reset(&mut self) {}
    /// Gain applied to raw samples before output; keeps pause trimming conservative.
    fn quiet_gain(&self) -> f32 {
        1.0
    }
}

/// Mono is duplicated to both outputs; stereo stays independent; additional
/// channels (no studio processor available) fall back to the first two.
#[derive(Default)]
pub struct Passthrough;

impl Processor for Passthrough {
    fn process(&mut self, input: &[i16], channels: usize, _flags: u8, gain: f32, out: &mut [f32]) {
        let frames = input.len() / channels;
        for f in 0..frames {
            let l = input[f * channels] as f32 * gain;
            let r = if channels == 1 { l } else { input[f * channels + 1] as f32 * gain };
            out[f * 2] = l;
            out[f * 2 + 1] = r;
        }
    }
}

pub struct Playout {
    pub jitter: JitterBuffer,
    passthrough: Passthrough,
    studio: Option<Box<dyn Processor>>,
    current: Vec<f32>,
    position: usize,
    using_studio: bool,
    pub gain: f32,
    pub muted: bool,
    peak: i32,
    /// Output frames that carried no audio while the stream was active.
    pub silent_frames: u64,
}

impl Playout {
    pub fn new(target_ms: u32, adaptive: bool, studio: Option<Box<dyn Processor>>) -> Self {
        Playout {
            jitter: JitterBuffer::new(target_ms, adaptive),
            passthrough: Passthrough,
            studio,
            current: vec![0.0; 480],
            position: 480,
            using_studio: false,
            gain: 1.0,
            muted: false,
            peak: 0,
            silent_frames: 0,
        }
    }

    pub fn peak(&self) -> i32 {
        self.peak
    }

    pub fn studio_active(&self) -> bool {
        self.using_studio
    }

    pub fn reset(&mut self) {
        self.jitter.reset();
        self.current.fill(0.0);
        self.position = self.current.len();
        self.peak = 0;
        if let Some(s) = self.studio.as_mut() {
            s.reset();
        }
    }

    fn gain(&self) -> f32 {
        if self.gain.is_finite() {
            self.gain.clamp(0.0, 10.0)
        } else {
            1.0
        }
    }

    /// Fill `out` (interleaved stereo PCM16) completely.
    pub fn read(&mut self, out: &mut [i16]) {
        assert!(out.len() % 2 == 0, "stereo PCM requires whole frames");
        self.jitter.observe_callback(out.len() / 2);
        self.peak = 0;
        let muted = self.muted;
        for sample in out.iter_mut() {
            if self.position == self.current.len() {
                self.next_packet();
            }
            let v = self.current[self.position];
            self.position += 1;
            let s = if muted { 0 } else { v.clamp(-32768.0, 32767.0) as i16 };
            self.peak = self.peak.max((s as i32).abs());
            *sample = s;
        }
    }

    fn next_packet(&mut self) {
        let n = self.current.len();
        let (last_l, last_r) = (self.current[n - 2], self.current[n - 1]);
        self.position = 0;
        let gain = self.gain();
        match self.jitter.next() {
            Next::Idle => {
                // Silence in the stream's own packet size keeps reads aligned.
                self.current.resize(self.jitter.packet_frames() * 2, 0.0);
                self.current.fill(0.0);
                self.silent_frames += (self.current.len() / 2) as u64;
            }
            Next::Packet { packet, fade_in } => {
                let frames = packet.frames();
                if self.current.len() != frames * 2 {
                    self.current.resize(frames * 2, 0.0);
                }
                let channels = packet.channels as usize;
                let studio = packet.flags & flags::STUDIO != 0 && self.studio.is_some();
                if studio != self.using_studio {
                    if let Some(s) = self.studio.as_mut() {
                        s.reset();
                    }
                    self.using_studio = studio;
                }
                if studio {
                    let p = self.studio.as_mut().unwrap();
                    p.process(&packet.samples, channels, packet.flags, gain, &mut self.current);
                    self.jitter.quiet_gain = p.quiet_gain() * gain.max(1.0);
                } else {
                    self.passthrough.process(&packet.samples, channels, packet.flags, gain, &mut self.current);
                    self.jitter.quiet_gain = gain;
                }
                if fade_in {
                    for f in 0..48.min(frames) {
                        let k = f as f32 / 48.0;
                        self.current[f * 2] *= k;
                        self.current[f * 2 + 1] *= k;
                    }
                }
            }
            Next::Conceal => {
                let frames = self.current.len() / 2;
                let tail = self.using_studio && self.studio.as_mut().is_some_and(|s| s.conceal(&mut self.current));
                for f in 0..frames {
                    let k = (frames - f - 1) as f32 / frames as f32;
                    if tail {
                        self.current[f * 2] *= k;
                        self.current[f * 2 + 1] *= k;
                    } else {
                        self.current[f * 2] = (last_l * k).trunc();
                        self.current[f * 2 + 1] = (last_r * k).trunc();
                    }
                }
                self.silent_frames += frames as u64 / 2;
            }
        }
    }
}
