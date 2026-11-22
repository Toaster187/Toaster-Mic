//! Toaster Mic UDP protocol v3. All multi-byte fields are little-endian.
//!
//! Every datagram: 36-byte header, optional payload, 16-byte tag.
//! Audio payloads are AES-256-GCM encrypted (nonce = header bytes 8..20,
//! the full header is AAD). Control payloads are authenticated with the
//! first 16 bytes of HMAC-SHA256 over header and payload.

use aes_gcm::aead::{AeadInPlace, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce, Tag};
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const MAGIC: [u8; 4] = *b"TMIC";
pub const VERSION: u8 = 3;
pub const RATE: u32 = 48_000;
pub const HEADER: usize = 36;
pub const TAG: usize = 16;
/// Largest UDP payload that never fragments on a 1500-byte Ethernet/WLAN MTU.
pub const MAX_DATAGRAM: usize = 1472;
pub const MAX_CHANNELS: u8 = 4;
pub const MAX_CONTROL_PAYLOAD: usize = 128;

pub const HELLO: u8 = 1;
pub const AUDIO: u8 = 2;
pub const STOP: u8 = 3;
/// PC -> phone over an established session: masked 32-byte pairing secret.
pub const PAIR: u8 = 4;
/// PC -> idle phone: embedded key-exchange offer, authenticated with the pairing secret.
pub const WAKE: u8 = 5;
/// Phone -> paired PC: "I am reachable at this address", authenticated with the pairing secret.
pub const ANNOUNCE: u8 = 6;

/// Default UDP port of the phone.
pub const PHONE_PORT: u16 = 8125;
/// Fixed UDP port on which a paired PC listens for ANNOUNCE.
pub const ANNOUNCE_PORT: u16 = 8126;

pub mod flags {
    pub const MUTED: u8 = 1;
    pub const STUDIO: u8 = 2;
    pub const PROFILE_SHIFT: u8 = 2;
    pub const PROFILE_MASK: u8 = 0b1100;
    /// Reserved: last channel carries a playback reference (future echo filter).
    pub const REFERENCE: u8 = 16;
    /// First seconds after an Always-On wake-up.
    pub const FAST_START: u8 = 32;
}

/// Sender latency profile. Larger bundles wake CPU and radio less often.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Profile {
    #[default]
    LowLatency = 0,
    Balanced = 1,
    Battery = 2,
}

impl Profile {
    pub fn from_flags(flags: u8) -> Self {
        match (flags & flags::PROFILE_MASK) >> flags::PROFILE_SHIFT {
            1 => Profile::Balanced,
            2 => Profile::Battery,
            _ => Profile::LowLatency,
        }
    }
    pub fn from_index(index: u8) -> Self {
        match index {
            1 => Profile::Balanced,
            2 => Profile::Battery,
            _ => Profile::LowLatency,
        }
    }
    pub fn bits(self) -> u8 {
        (self as u8) << flags::PROFILE_SHIFT
    }
    /// Audio duration in milliseconds the sender collects before one send burst.
    pub fn bundle_ms(self) -> u32 {
        match self {
            Profile::LowLatency => 0,
            Profile::Balanced => 10,
            Profile::Battery => 20,
        }
    }
    /// Minimum receiver reserve that absorbs one send burst without an underrun.
    pub fn min_reserve_ms(self) -> u32 {
        match self {
            Profile::LowLatency => 5,
            Profile::Balanced => 15,
            Profile::Battery => 25,
        }
    }
}

/// Frames per audio packet: 5 ms for mono/stereo, 3.33 ms for 3-4 channels
/// so that every datagram stays below [`MAX_DATAGRAM`].
pub const fn frames_for(channels: u8) -> u16 {
    if channels <= 2 {
        240
    } else {
        160
    }
}

pub const fn audio_payload_len(channels: u8) -> usize {
    frames_for(channels) as usize * channels as usize * 2
}

pub const fn audio_len(channels: u8) -> usize {
    HEADER + audio_payload_len(channels) + TAG
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Header {
    pub kind: u8,
    pub channels: u8,
    pub flags: u8,
    pub session: u64,
    pub sequence: u32,
    pub frames: u16,
    /// Control payload length; 0 for audio.
    pub payload: u16,
    /// Monotonic counter for WAKE/ANNOUNCE/PAIR, otherwise 0.
    pub counter: u64,
}

impl Header {
    pub fn control(kind: u8, session: u64, counter: u64, payload: usize) -> Self {
        Header { kind, channels: 0, flags: 0, session, sequence: 0, frames: 0, payload: payload as u16, counter }
    }
    pub fn audio(channels: u8, flags: u8, session: u64, sequence: u32) -> Self {
        Header { kind: AUDIO, channels, flags, session, sequence, frames: frames_for(channels), payload: 0, counter: 0 }
    }
    pub fn datagram_len(&self) -> usize {
        if self.kind == AUDIO {
            HEADER + self.frames as usize * self.channels as usize * 2 + TAG
        } else {
            HEADER + self.payload as usize + TAG
        }
    }
    pub fn write(&self, out: &mut [u8]) {
        out[0..4].copy_from_slice(&MAGIC);
        out[4] = VERSION;
        out[5] = self.kind;
        out[6] = self.channels;
        out[7] = self.flags;
        out[8..16].copy_from_slice(&self.session.to_le_bytes());
        out[16..20].copy_from_slice(&self.sequence.to_le_bytes());
        out[20..24].copy_from_slice(&RATE.to_le_bytes());
        out[24..26].copy_from_slice(&self.frames.to_le_bytes());
        out[26..28].copy_from_slice(&self.payload.to_le_bytes());
        out[28..36].copy_from_slice(&self.counter.to_le_bytes());
    }
}

fn u16_at(d: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([d[i], d[i + 1]])
}
fn u32_at(d: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(d[i..i + 4].try_into().unwrap())
}
fn u64_at(d: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(d[i..i + 8].try_into().unwrap())
}

/// Validates framing only (no authentication). Rejects every length,
/// type, rate and channel combination that a real sender never produces.
pub fn parse_header(data: &[u8]) -> Option<Header> {
    if data.len() < HEADER + TAG || data[0..4] != MAGIC || data[4] != VERSION || u32_at(data, 20) != RATE {
        return None;
    }
    let h = Header {
        kind: data[5],
        channels: data[6],
        flags: data[7],
        session: u64_at(data, 8),
        sequence: u32_at(data, 16),
        frames: u16_at(data, 24),
        payload: u16_at(data, 26),
        counter: u64_at(data, 28),
    };
    match h.kind {
        AUDIO => {
            if h.channels == 0 || h.channels > MAX_CHANNELS || h.frames != frames_for(h.channels) || h.payload != 0 {
                return None;
            }
        }
        HELLO | STOP | PAIR | WAKE | ANNOUNCE => {
            if h.channels != 0 || h.frames != 0 || h.payload as usize > MAX_CONTROL_PAYLOAD {
                return None;
            }
        }
        _ => return None,
    }
    (data.len() == h.datagram_len()).then_some(h)
}

type HmacSha256 = Hmac<Sha256>;

/// HMAC key for control datagrams (session key or pairing secret).
#[derive(Clone)]
pub struct ControlKey {
    mac: HmacSha256,
}

impl ControlKey {
    pub fn new(key: &[u8]) -> Self {
        ControlKey { mac: <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC accepts any key length") }
    }

    /// Writes a complete control datagram into `out` and returns its length.
    pub fn seal_into(&self, header: Header, payload: &[u8], out: &mut [u8]) -> usize {
        debug_assert!(header.kind != AUDIO && payload.len() == header.payload as usize);
        let len = header.datagram_len();
        header.write(out);
        out[HEADER..HEADER + payload.len()].copy_from_slice(payload);
        let mut mac = self.mac.clone();
        mac.update(&out[..len - TAG]);
        let tag = mac.finalize().into_bytes();
        out[len - TAG..len].copy_from_slice(&tag[..TAG]);
        len
    }

    pub fn seal(&self, kind: u8, session: u64, counter: u64, payload: &[u8]) -> Vec<u8> {
        let header = Header::control(kind, session, counter, payload.len());
        let mut out = vec![0u8; header.datagram_len()];
        self.seal_into(header, payload, &mut out);
        out
    }

    /// Returns header and payload of an authentic control datagram.
    pub fn open<'a>(&self, data: &'a [u8]) -> Option<(Header, &'a [u8])> {
        let h = parse_header(data)?;
        if h.kind == AUDIO {
            return None;
        }
        let mut mac = self.mac.clone();
        mac.update(&data[..data.len() - TAG]);
        mac.verify_truncated_left(&data[data.len() - TAG..]).ok()?;
        Some((h, &data[HEADER..HEADER + h.payload as usize]))
    }

    /// 32-byte pseudo-random mask bound to a label, session and counter.
    pub fn mask(&self, label: &[u8], session: u64, counter: u64) -> [u8; 32] {
        let mut mac = self.mac.clone();
        mac.update(label);
        mac.update(&session.to_le_bytes());
        mac.update(&counter.to_le_bytes());
        mac.finalize().into_bytes().into()
    }
}

/// Per-session keys. AES-GCM state is expanded once, not per packet.
#[derive(Clone)]
pub struct SessionKey {
    gcm: Aes256Gcm,
    control: ControlKey,
}

impl SessionKey {
    pub fn new(key: &[u8; 32]) -> Self {
        SessionKey { gcm: Aes256Gcm::new(key.into()), control: ControlKey::new(key) }
    }

    pub fn control(&self) -> &ControlKey {
        &self.control
    }

    /// Encrypts in place. `buf[HEADER..HEADER + audio_payload_len(channels)]`
    /// must already hold little-endian PCM16. Returns the datagram length.
    pub fn seal_audio(&self, buf: &mut [u8], channels: u8, flags: u8, session: u64, sequence: u32) -> usize {
        let header = Header::audio(channels, flags, session, sequence);
        let len = header.datagram_len();
        header.write(buf);
        let (head, rest) = buf[..len].split_at_mut(HEADER);
        let (pcm, tag_out) = rest.split_at_mut(len - HEADER - TAG);
        let tag = self
            .gcm
            .encrypt_in_place_detached(Nonce::from_slice(&head[8..20]), head, pcm)
            .expect("PCM payload is far below the AES-GCM limit");
        tag_out.copy_from_slice(&tag);
        len
    }

    /// Decrypts an audio datagram in place. On success the PCM16 payload is
    /// `data[HEADER..data.len() - TAG]`.
    pub fn open_audio(&self, data: &mut [u8]) -> Option<Header> {
        let h = parse_header(data)?;
        if h.kind != AUDIO {
            return None;
        }
        let len = data.len();
        let (head, rest) = data.split_at_mut(HEADER);
        let (pcm, tag) = rest.split_at_mut(len - HEADER - TAG);
        let tag = *Tag::from_slice(tag);
        self.gcm
            .decrypt_in_place_detached(Nonce::from_slice(&head[8..20]), head, pcm, &tag)
            .ok()?;
        Some(h)
    }
}

/// PAIR payload: the pairing secret XOR a mask only the session peers can compute.
pub fn pair_payload(session_key: &SessionKey, session: u64, counter: u64, secret: &[u8; 32]) -> [u8; 32] {
    let mask = session_key.control().mask(b"TMIC-PAIR", session, counter);
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = secret[i] ^ mask[i];
    }
    out
}

/// Write interleaved samples as little-endian PCM16.
pub fn write_pcm(samples: &[i16], out: &mut [u8]) {
    for (s, b) in samples.iter().zip(out.chunks_exact_mut(2)) {
        b.copy_from_slice(&s.to_le_bytes());
    }
}

pub fn read_pcm(bytes: &[u8], out: &mut [i16]) {
    for (b, s) in bytes.chunks_exact(2).zip(out.iter_mut()) {
        *s = i16::from_le_bytes([b[0], b[1]]);
    }
}
