//! Small status file the engine rewrites about ten times per second while it
//! runs (and only on changes while idle); the UI reads it while visible.
//! A sequence number at both ends detects torn reads.

use std::fs::File;
use std::path::{Path, PathBuf};

const MAGIC: u32 = 0x544D_5331; // "TMS1"
pub const SIZE: usize = 320;
const MESSAGE: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum StatusCode {
    #[default]
    Stopped = 0,
    Starting = 1,
    /// Capturing, waiting for a PC.
    Ready = 2,
    Streaming = 3,
    /// Always-On: idle, waiting for a WAKE.
    Standby = 4,
    Error = 5,
}

impl StatusCode {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => StatusCode::Starting,
            2 => StatusCode::Ready,
            3 => StatusCode::Streaming,
            4 => StatusCode::Standby,
            5 => StatusCode::Error,
            _ => StatusCode::Stopped,
        }
    }
}

pub mod flag {
    pub const MUTED: u8 = 1;
    pub const LIVE: u8 = 2;
    pub const PAIRED: u8 = 4;
    pub const STUDIO: u8 = 8;
    pub const FAST_START: u8 = 16;
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Status {
    pub code: StatusCode,
    pub channels: u8,
    pub flags: u8,
    pub peak: u16,
    pub left_peak: u16,
    pub right_peak: u16,
    pub peer: [u8; 4],
    pub packets: u32,
    /// Process id of the writing engine (0 = unknown); lets the UI detect a
    /// killed engine whose last status is still in the file.
    pub pid: u32,
    pub message: String,
}

impl Status {
    pub fn path(files: &Path) -> PathBuf {
        files.join("status.bin")
    }

    pub fn encode(&self, seq: u32) -> [u8; SIZE] {
        let mut b = [0u8; SIZE];
        b[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        b[4..8].copy_from_slice(&seq.to_le_bytes());
        b[8] = self.code as u8;
        b[9] = self.channels;
        b[10] = self.flags;
        b[12..14].copy_from_slice(&self.peak.to_le_bytes());
        b[14..16].copy_from_slice(&self.left_peak.to_le_bytes());
        b[16..18].copy_from_slice(&self.right_peak.to_le_bytes());
        b[18..22].copy_from_slice(&self.peer);
        b[22..26].copy_from_slice(&self.packets.to_le_bytes());
        let msg = self.message.as_bytes();
        let mut n = msg.len().min(MESSAGE);
        while !self.message.is_char_boundary(n) {
            n -= 1;
        }
        b[26..28].copy_from_slice(&(n as u16).to_le_bytes());
        b[28..28 + n].copy_from_slice(&msg[..n]);
        b[SIZE - 8..SIZE - 4].copy_from_slice(&self.pid.to_le_bytes());
        b[SIZE - 4..].copy_from_slice(&seq.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Option<Status> {
        if b.len() < SIZE || b[0..4] != MAGIC.to_le_bytes() || b[4..8] != b[SIZE - 4..SIZE] {
            return None;
        }
        let n = (u16::from_le_bytes([b[26], b[27]]) as usize).min(MESSAGE);
        Some(Status {
            code: StatusCode::from_u8(b[8]),
            channels: b[9],
            flags: b[10],
            peak: u16::from_le_bytes([b[12], b[13]]),
            left_peak: u16::from_le_bytes([b[14], b[15]]),
            right_peak: u16::from_le_bytes([b[16], b[17]]),
            peer: b[18..22].try_into().unwrap(),
            packets: u32::from_le_bytes(b[22..26].try_into().unwrap()),
            pid: u32::from_le_bytes(b[SIZE - 8..SIZE - 4].try_into().unwrap()),
            message: String::from_utf8_lossy(&b[28..28 + n]).into_owned(),
        })
    }

    /// Whether the engine that wrote this status is still running.
    #[cfg(unix)]
    pub fn writer_alive(&self) -> bool {
        self.pid != 0 && std::path::Path::new(&format!("/proc/{}", self.pid)).exists()
    }

    pub fn read(files: &Path) -> Option<Status> {
        for _ in 0..3 {
            if let Some(s) = std::fs::read(Self::path(files)).ok().and_then(|b| Status::decode(&b)) {
                return Some(s);
            }
        }
        None
    }
}

/// Writer side: keeps the file open and overwrites it in place.
pub struct StatusWriter {
    file: Option<File>,
    seq: u32,
    last: Status,
}

impl StatusWriter {
    pub fn open(files: &Path) -> StatusWriter {
        let file = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(Status::path(files)).ok();
        StatusWriter { file, seq: 0, last: Status { message: "\u{0}".into(), ..Default::default() } }
    }

    /// Writes only when something changed.
    pub fn write(&mut self, status: &Status) {
        if *status == self.last {
            return;
        }
        self.seq = self.seq.wrapping_add(1);
        let bytes = status.encode(self.seq);
        if let Some(f) = &self.file {
            write_at(f, &bytes);
        }
        self.last = status.clone();
    }
}

#[cfg(unix)]
fn write_at(f: &File, b: &[u8]) {
    use std::os::unix::fs::FileExt;
    let _ = f.write_all_at(b, 0);
}

#[cfg(windows)]
fn write_at(f: &File, b: &[u8]) {
    use std::os::windows::fs::FileExt;
    let _ = f.seek_write(b, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_torn_detection() {
        let s = Status { code: StatusCode::Streaming, channels: 3, flags: flag::STUDIO, peak: 1234, peer: [192, 168, 1, 5], packets: 99, pid: 4321, message: "Überträgt · Studio".into(), ..Default::default() };
        let b = s.encode(7);
        assert_eq!(Status::decode(&b), Some(s.clone()));
        let mut torn = b;
        torn[SIZE - 1] ^= 1;
        assert!(Status::decode(&torn).is_none());
        let long = Status { message: "ä".repeat(300), ..s };
        assert!(Status::decode(&long.encode(1)).unwrap().message.len() <= MESSAGE);
    }
}
