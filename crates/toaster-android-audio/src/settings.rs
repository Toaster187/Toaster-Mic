//! `settings.json` (written by the UI) and `pairing.json` (written by the
//! engine) in the app's private files directory.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use toaster_core::phone::Pairing;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Mono,
    Stereo,
    Studio,
}

/// Physical microphone map from the guided calibration, bound to the OS build.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Calibration {
    pub build: String,
    pub channels: usize,
    /// Raw channel index of top, bottom and (if present) rear microphone.
    pub top: i32,
    pub bottom: i32,
    pub rear: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PhoneSettings {
    pub port: u16,
    pub mode: Mode,
    /// Capture gain in percent, 0..=200.
    pub gain: u32,
    /// 0: left = bottom/first, 1: swapped.
    pub left: u8,
    /// Preferred input device id for mono/stereo (0 = automatic).
    pub device: i32,
    /// Mono microphone: 0 automatic, 1 bottom, 2 top, 3 rear (needs the guided assignment).
    pub mono_mic: u8,
    /// 0 lowest latency, 1 balanced, 2 battery.
    pub profile: u8,
    pub muted: bool,
    /// Do-not-disturb while transmitting.
    pub live: bool,
    pub always_on: bool,
    pub calibration: Option<Calibration>,
}

impl Default for PhoneSettings {
    fn default() -> Self {
        PhoneSettings {
            port: toaster_core::wire::PHONE_PORT,
            mode: Mode::Mono,
            gain: 100,
            left: 0,
            device: 0,
            mono_mic: 0,
            profile: 0,
            muted: false,
            live: false,
            always_on: false,
            calibration: None,
        }
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(&tmp, path)
}

impl PhoneSettings {
    pub fn path(files: &Path) -> PathBuf {
        files.join("settings.json")
    }
    pub fn load(files: &Path) -> PhoneSettings {
        let mut s: PhoneSettings = read_json(&Self::path(files)).unwrap_or_default();
        if s.port < 1024 {
            s.port = toaster_core::wire::PHONE_PORT;
        }
        s.gain = s.gain.min(200);
        s.left = s.left.min(1);
        s.profile = s.profile.min(2);
        s.mono_mic = s.mono_mic.min(3);
        s
    }
    pub fn save(&self, files: &Path) -> std::io::Result<()> {
        write_json(&Self::path(files), self)
    }
    pub fn calibrated_for(&self, build: &str, channels: usize) -> Option<&Calibration> {
        self.calibration.as_ref().filter(|c| c.build == build && c.channels == channels && c.valid())
    }
}

impl Calibration {
    pub fn valid(&self) -> bool {
        let idx: Vec<i32> = if self.channels >= 3 { vec![self.top, self.bottom, self.rear] } else { vec![self.top, self.bottom] };
        toaster_core::calibration::distinct(&idx, self.channels)
    }
}

/// Engine-owned: pairing with the PC plus the ANNOUNCE counter.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct PairingFile {
    pub secret: String,
    pub last_counter: u64,
    pub pc: Option<String>,
    pub announce_counter: u64,
}

impl PairingFile {
    pub fn path(files: &Path) -> PathBuf {
        files.join("pairing.json")
    }
    pub fn load(files: &Path) -> PairingFile {
        read_json(&Self::path(files)).unwrap_or_default()
    }
    pub fn save(&self, files: &Path) -> std::io::Result<()> {
        write_json(&Self::path(files), self)
    }
    pub fn pairing(&self) -> Option<Pairing> {
        let bytes: Vec<u8> = (0..self.secret.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(self.secret.get(i..i + 2)?, 16).ok())
            .collect::<Option<Vec<u8>>>()?;
        let secret: [u8; 32] = bytes.try_into().ok()?;
        Some(Pairing { secret, last_counter: self.last_counter, pc: self.pc.as_ref().and_then(|p| p.parse().ok()) })
    }
    pub fn set(&mut self, p: Option<&Pairing>) {
        match p {
            Some(p) => {
                self.secret = p.secret.iter().map(|b| format!("{b:02x}")).collect();
                self.last_counter = p.last_counter;
                self.pc = p.pc.map(|ip| ip.to_string());
            }
            None => *self = PairingFile { announce_counter: self.announce_counter, ..Default::default() },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_and_pairing_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tm-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = PhoneSettings { mode: Mode::Studio, live: true, gain: 150, ..Default::default() };
        s.save(&dir).unwrap();
        assert_eq!(PhoneSettings::load(&dir), s);
        let mut pf = PairingFile::default();
        let p = Pairing { secret: [7; 32], last_counter: 42, pc: Some("192.168.1.2".parse().unwrap()) };
        pf.set(Some(&p));
        pf.save(&dir).unwrap();
        assert_eq!(PairingFile::load(&dir).pairing(), Some(p));
        pf.set(None);
        assert!(pf.pairing().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn calibration_validity() {
        let c = Calibration { build: "b".into(), channels: 3, top: 2, bottom: 0, rear: 1 };
        assert!(c.valid());
        let s = PhoneSettings { calibration: Some(c.clone()), ..Default::default() };
        assert!(s.calibrated_for("b", 3).is_some());
        assert!(s.calibrated_for("other", 3).is_none());
        assert!(!Calibration { rear: 2, ..c }.valid());
    }
}
