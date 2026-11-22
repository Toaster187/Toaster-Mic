//! Ephemeral P-256 key exchange (`TMK2`). Gives confidentiality without a
//! manually entered secret; it does not authenticate against an active
//! LAN attacker (same threat model as protocol v2).
//!
//! Offer layout (80 bytes): `TMK2`, type (1 = PC, 2 = phone), three zero
//! bytes, 64-bit session id (LE), public X (32 bytes BE), public Y (32 bytes BE).

use p256::ecdh::EphemeralSecret;
use p256::elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint};
use p256::{EncodedPoint, FieldBytes, PublicKey};
use rand_core::OsRng;
use sha2::{Digest, Sha256};

pub const OFFER_LEN: usize = 80;
pub const MAGIC: [u8; 4] = *b"TMK2";
pub const FROM_PC: u8 = 1;
pub const FROM_PHONE: u8 = 2;

pub struct KeyExchange {
    secret: EphemeralSecret,
    xy: [u8; 64],
}

impl Default for KeyExchange {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyExchange {
    pub fn new() -> Self {
        let secret = EphemeralSecret::random(&mut OsRng);
        let point = secret.public_key().to_encoded_point(false);
        let mut xy = [0u8; 64];
        xy[..32].copy_from_slice(point.x().expect("uncompressed point"));
        xy[32..].copy_from_slice(point.y().expect("uncompressed point"));
        KeyExchange { secret, xy }
    }

    pub fn offer(&self, kind: u8, session: u64) -> [u8; OFFER_LEN] {
        let mut out = [0u8; OFFER_LEN];
        out[..4].copy_from_slice(&MAGIC);
        out[4] = kind;
        out[8..16].copy_from_slice(&session.to_le_bytes());
        out[16..].copy_from_slice(&self.xy);
        out
    }

    /// SHA-256 over the 32-byte ECDH x coordinate. `None` for malformed
    /// offers and points that are not on the curve.
    pub fn derive(&self, offer: &[u8]) -> Option<[u8; 32]> {
        if offer.len() != OFFER_LEN {
            return None;
        }
        let point = EncodedPoint::from_affine_coordinates(
            FieldBytes::from_slice(&offer[16..48]),
            FieldBytes::from_slice(&offer[48..80]),
            false,
        );
        let remote: PublicKey = Option::from(PublicKey::from_encoded_point(&point))?;
        let shared = self.secret.diffie_hellman(&remote);
        Some(Sha256::digest(shared.raw_secret_bytes()).into())
    }
}

pub fn is_offer(data: &[u8], kind: u8) -> bool {
    data.len() == OFFER_LEN && data[..4] == MAGIC && data[4] == kind && data[5..8] == [0, 0, 0]
}

pub fn offer_session(data: &[u8]) -> u64 {
    u64::from_le_bytes(data[8..16].try_into().unwrap())
}
