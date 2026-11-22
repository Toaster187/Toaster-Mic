//! GCC-PHAT time-difference estimation between two microphones.

use std::sync::Arc;

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

pub const FFT: usize = 1024;
/// Largest plausible delay between phone microphones (~20 cm at 48 kHz).
pub const MAX_LAG: usize = 32;

pub struct Gcc {
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    a: Vec<f32>,
    b: Vec<f32>,
    fa: Vec<Complex32>,
    fb: Vec<Complex32>,
    cross: Vec<f32>,
    scratch_f: Vec<Complex32>,
    scratch_i: Vec<Complex32>,
}

impl Default for Gcc {
    fn default() -> Self {
        Self::new()
    }
}

impl Gcc {
    pub fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(FFT);
        let inv = planner.plan_fft_inverse(FFT);
        Gcc {
            a: fwd.make_input_vec(),
            b: fwd.make_input_vec(),
            fa: fwd.make_output_vec(),
            fb: fwd.make_output_vec(),
            cross: inv.make_output_vec(),
            scratch_f: fwd.make_scratch_vec(),
            scratch_i: inv.make_scratch_vec(),
            fwd,
            inv,
        }
    }

    /// Delay (in samples, fractional) of `other` relative to `reference`:
    /// positive when `other` hears the source later. Returns the peak
    /// sharpness too (peak / mean), a confidence measure.
    pub fn delay(&mut self, reference: &[f32], other: &[f32]) -> (f32, f32) {
        let n = reference.len().min(other.len()).min(FFT - MAX_LAG);
        self.a.fill(0.0);
        self.b.fill(0.0);
        // Hann window against edge effects.
        for i in 0..n {
            let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos();
            self.a[i] = reference[i] * w;
            self.b[i] = other[i] * w;
        }
        let _ = self.fwd.process_with_scratch(&mut self.a, &mut self.fa, &mut self.scratch_f);
        let _ = self.fwd.process_with_scratch(&mut self.b, &mut self.fb, &mut self.scratch_f);
        for (x, y) in self.fa.iter_mut().zip(&self.fb) {
            // Partial phase transform (beta = 0.75): sharp peaks for speech
            // without letting noise-only bins dominate.
            let c = y * x.conj();
            let m = c.norm();
            *x = if m > 1e-12 { c / m.powf(0.75) } else { Complex32::new(0.0, 0.0) };
        }
        self.fa[0].im = 0.0;
        let last = self.fa.len() - 1;
        self.fa[last].im = 0.0;
        let _ = self.inv.process_with_scratch(&mut self.fa, &mut self.cross, &mut self.scratch_i);
        let at = |lag: isize| -> f32 {
            let idx = if lag >= 0 { lag as usize } else { (FFT as isize + lag) as usize };
            self.cross[idx]
        };
        let (mut best, mut best_lag) = (f32::MIN, 0isize);
        let mut sum = 0.0;
        for lag in -(MAX_LAG as isize)..=MAX_LAG as isize {
            let v = at(lag);
            sum += v.abs();
            if v > best {
                best = v;
                best_lag = lag;
            }
        }
        let mean = sum / (2 * MAX_LAG + 1) as f32;
        // Parabolic interpolation for sub-sample precision.
        let (l, c, r) = (at(best_lag - 1), best, at(best_lag + 1));
        let denom = l - 2.0 * c + r;
        let frac = if denom.abs() > 1e-9 { (0.5 * (l - r) / denom).clamp(-0.5, 0.5) } else { 0.0 };
        (best_lag as f32 + frac, if mean > 0.0 { best / mean } else { 0.0 })
    }
}
