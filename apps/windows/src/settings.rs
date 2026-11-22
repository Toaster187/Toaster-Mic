//! Persistent settings in `%LOCALAPPDATA%\ToasterMic\settings.json`.
//! Field names stay compatible with the 0.4 WPF app.

use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct Pairing {
    /// 32-byte secret, hex encoded.
    pub secret: String,
    /// Next unused WAKE counter block.
    pub counter: u64,
    /// Highest accepted ANNOUNCE counter from the phone.
    pub announce_counter: u64,
    /// The phone confirmed it stored the secret.
    pub acked: bool,
}

impl Default for Pairing {
    fn default() -> Self {
        Pairing { secret: String::new(), counter: 1, announce_counter: 0, acked: false }
    }
}

impl Pairing {
    pub fn generate() -> Self {
        let secret: [u8; 32] = toaster_core::random_bytes();
        Pairing { secret: hex(&secret), ..Default::default() }
    }
    pub fn secret_bytes(&self) -> Option<[u8; 32]> {
        let bytes = unhex(&self.secret)?;
        bytes.try_into().ok()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct Settings {
    pub host: String,
    #[serde(deserialize_with = "port_from_any")]
    pub port: u16,
    /// Output volume in percent, 0..=1000.
    pub gain: f64,
    /// Buffer preset index: from 5 / 15 / 30 / 60 ms.
    pub buffer: usize,
    /// WASAPI endpoint id of the output (CABLE Input).
    pub output: Option<String>,
    /// Studio noise suppression: 0 off, 1 medium, 2 strong.
    pub noise: u8,
    pub always_on: bool,
    pub autostart: bool,
    pub auto_connect: bool,
    pub muted: bool,
    pub pairing: Option<Pairing>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            host: "192.168.1.100".into(),
            port: toaster_core::wire::PHONE_PORT,
            gain: 100.0,
            buffer: 1,
            output: None,
            noise: 1,
            always_on: false,
            autostart: false,
            auto_connect: false,
            muted: false,
            pairing: None,
        }
    }
}

pub const BUFFER_PRESETS_MS: [u32; 4] = [5, 15, 30, 60];

impl Settings {
    pub fn buffer_ms(&self) -> u32 {
        BUFFER_PRESETS_MS[self.buffer.min(3)]
    }

    pub fn path() -> PathBuf {
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        base.join("ToasterMic").join("settings.json")
    }

    pub fn load() -> Settings {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
            .map(|mut s| {
                if !s.gain.is_finite() {
                    s.gain = 100.0;
                }
                s.gain = s.gain.clamp(0.0, 1000.0);
                s.buffer = s.buffer.min(3);
                s.noise = s.noise.min(2);
                if s.port < 1024 {
                    s.port = toaster_core::wire::PHONE_PORT;
                }
                s
            })
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            // Write-then-rename keeps the old file if the process dies mid-write.
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
}

fn port_from_any<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Any {
        Num(u64),
        Text(String),
    }
    Ok(match Any::deserialize(d)? {
        Any::Num(n) => u16::try_from(n).unwrap_or(toaster_core::wire::PHONE_PORT),
        Any::Text(t) => t.trim().parse().unwrap_or(toaster_core::wire::PHONE_PORT),
    })
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_04_settings_file() {
        let old = r#"{"Host":"192.168.0.7","Port":"9000","Gain":250,"Buffer":2,"Output":"{0.0.0.00000000}.{abc}"}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!((s.host.as_str(), s.port, s.gain, s.buffer), ("192.168.0.7", 9000, 250.0, 2));
        assert!(!s.always_on && s.pairing.is_none());
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn pairing_secret_roundtrip() {
        let p = Pairing::generate();
        assert_eq!(p.secret.len(), 64);
        assert_eq!(hex(&p.secret_bytes().unwrap()), p.secret);
    }
}
