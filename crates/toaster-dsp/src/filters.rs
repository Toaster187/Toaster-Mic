//! Small real-time building blocks. No allocation after construction.

use std::f32::consts::PI;

/// 2nd-order Butterworth high-pass (RBJ cookbook), transposed direct form II.
#[derive(Clone, Copy, Debug)]
pub struct HighPass {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl HighPass {
    pub fn new(cutoff: f32, rate: f32) -> Self {
        let w = 2.0 * PI * cutoff / rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin * std::f32::consts::FRAC_1_SQRT_2; // Q = 1/sqrt(2)
        let a0 = 1.0 + alpha;
        HighPass {
            b0: (1.0 + cos) / 2.0 / a0,
            b1: -(1.0 + cos) / a0,
            b2: (1.0 + cos) / 2.0 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

pub const SINC_TAPS: usize = 16;

/// Ring buffer that can be read at a fractional delay (windowed sinc).
#[derive(Clone, Debug)]
pub struct DelayLine {
    buf: Vec<f32>,
    pos: usize,
}

impl DelayLine {
    pub fn new(capacity: usize) -> Self {
        DelayLine { buf: vec![0.0; capacity.next_power_of_two()], pos: 0 }
    }
    #[inline]
    pub fn push(&mut self, x: f32) {
        self.pos = (self.pos + 1) & (self.buf.len() - 1);
        self.buf[self.pos] = x;
    }
    #[inline]
    pub fn at(&self, delay: usize) -> f32 {
        self.buf[(self.pos + self.buf.len() - delay) & (self.buf.len() - 1)]
    }
    /// Sample `delay` samples ago, `delay` may be fractional (>= SINC_TAPS / 2).
    /// Uses precomputed windowed-sinc kernels in 1/64-sample steps.
    #[inline]
    pub fn read(&self, delay: f32) -> f32 {
        let whole = delay.floor();
        let step = ((delay - whole) * FRACTIONS as f32).round() as usize;
        let (d, step) = if step == FRACTIONS { (whole as isize + 1, 0) } else { (whole as isize, step) };
        if step == 0 {
            return self.at(d as usize);
        }
        let kernel = &kernels()[step];
        let half = (SINC_TAPS / 2) as isize;
        let mut acc = 0.0;
        for (i, w) in kernel.iter().enumerate() {
            acc += self.at((d - half + 1 + i as isize) as usize) * w;
        }
        acc
    }
    pub fn reset(&mut self) {
        self.buf.fill(0.0);
    }
}

const FRACTIONS: usize = 64;

fn kernels() -> &'static [[f32; SINC_TAPS]; FRACTIONS] {
    static K: std::sync::OnceLock<[[f32; SINC_TAPS]; FRACTIONS]> = std::sync::OnceLock::new();
    K.get_or_init(|| {
        let half = (SINC_TAPS / 2) as isize;
        std::array::from_fn(|step| {
            let frac = step as f32 / FRACTIONS as f32;
            let mut k = [0.0f32; SINC_TAPS];
            for (i, w) in k.iter_mut().enumerate() {
                let t = (i as isize - half + 1) as f32 - frac;
                let x = PI * t;
                let sinc = if t.abs() < 1e-6 { 1.0 } else { x.sin() / x };
                *w = sinc * (0.5 + 0.5 * (PI * t / half as f32).cos());
            }
            k
        })
    })
}

/// One-pole smoother coefficient for a time constant in seconds.
pub fn coeff(seconds: f32, rate: f32) -> f32 {
    (-1.0 / (seconds * rate)).exp()
}
