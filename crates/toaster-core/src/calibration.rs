//! Conservative level comparison for user-guided microphone identification.
//! Only RMS levels are compared; no audio is stored.

/// Index of the single channel that clearly dominates after subtracting the
/// background level, or `None` when the measurement is ambiguous.
pub fn identify(background: &[f64], signal: &[f64]) -> Option<usize> {
    if background.len() != signal.len() || background.len() < 2 {
        return None;
    }
    let (mut best, mut second, mut channel) = (0.0f64, 0.0f64, None);
    for i in 0..signal.len() {
        let (b, s) = (background[i], signal[i]);
        if !b.is_finite() || !s.is_finite() || b < 0.0 || s < 0.0 {
            return None;
        }
        let level = (s * s - b * b).max(0.0).sqrt();
        if level > best {
            second = best;
            best = level;
            channel = Some(i);
        } else {
            second = second.max(level);
        }
    }
    let c = channel?;
    if best < 40f64.max(background[c] * 3.0) || best < second * 1.6 {
        return None;
    }
    Some(c)
}

/// All indices valid for `channels` raw inputs and pairwise different.
pub fn distinct(indices: &[i32], channels: usize) -> bool {
    indices.iter().enumerate().all(|(i, &a)| {
        a >= 0 && (a as usize) < channels && indices[i + 1..].iter().all(|&b| b != a)
    })
}

/// Accumulates per-channel energy of interleaved PCM16.
#[derive(Clone, Debug)]
pub struct LevelMeter {
    energy: Vec<f64>,
    frames: u64,
}

impl LevelMeter {
    pub fn new(channels: usize) -> Self {
        LevelMeter { energy: vec![0.0; channels], frames: 0 }
    }
    pub fn add(&mut self, interleaved: &[i16]) {
        let ch = self.energy.len();
        for frame in interleaved.chunks_exact(ch) {
            for (e, &s) in self.energy.iter_mut().zip(frame) {
                *e += s as f64 * s as f64;
            }
        }
        self.frames += (interleaved.len() / ch) as u64;
    }
    pub fn rms(&self) -> Vec<f64> {
        let n = self.frames.max(1) as f64;
        self.energy.iter().map(|e| (e / n).sqrt()).collect()
    }
}
