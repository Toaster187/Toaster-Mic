//! Shared core of Toaster Mic: wire protocol v3, key exchange, jitter buffer
//! and playout. Used unchanged by the Android sender and the Windows receiver.

pub mod calibration;
pub mod jitter;
pub mod keyx;
pub mod lease;
pub mod phone;
pub mod playout;
pub mod sim;
pub mod wire;

pub use jitter::{AudioPacket, JitterBuffer, Stats};
pub use keyx::KeyExchange;
pub use lease::{now_ms, ConnectionLease};
pub use playout::{Passthrough, Playout, Processor};
pub use wire::{ControlKey, Header, Profile, SessionKey};

/// Random 64-bit value from the operating system.
pub fn random_u64() -> u64 {
    use rand_core::{OsRng, RngCore};
    OsRng.next_u64()
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    use rand_core::{OsRng, RngCore};
    let mut out = [0u8; N];
    OsRng.fill_bytes(&mut out);
    out
}
