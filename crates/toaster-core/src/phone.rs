//! Phone-side control state machine, independent of sockets and audio.
//! Shared by the Android engine and the desktop phone simulator.
//!
//! Rules (unchanged from 0.4 unless noted):
//! * A key-exchange offer is answered with an ephemeral reply; identical
//!   repeated offers get the identical reply. At most four new key pairs
//!   per second, a pending exchange is held for 3.5 s.
//! * Only an authenticated HELLO confirms key possession and starts audio.
//! * An active peer (HELLO within 3.5 s) is never taken over by another endpoint.
//! * New: with a pairing secret the phone accepts authenticated WAKE
//!   datagrams (embedded offer + strictly increasing counter) that start audio
//!   immediately. In standby only WAKE is accepted.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use crate::keyx::{self, KeyExchange, FROM_PC, FROM_PHONE, OFFER_LEN};
use crate::wire::{self, ControlKey, SessionKey, HELLO, PAIR, STOP, WAKE};

pub const LEASE_MS: u64 = 3_500;
const EXCHANGE_INTERVAL_MS: u64 = 250;

pub struct Peer {
    pub addr: SocketAddr,
    pub session: u64,
    pub key: SessionKey,
    request: [u8; OFFER_LEN],
    reply: [u8; OFFER_LEN],
    pub woken: bool,
}

impl Peer {
    fn new(addr: SocketAddr, request: &[u8]) -> Option<Peer> {
        let exchange = KeyExchange::new();
        let key = exchange.derive(request)?;
        let session = keyx::offer_session(request);
        Some(Peer {
            addr,
            session,
            key: SessionKey::new(&key),
            request: request.try_into().ok()?,
            reply: exchange.offer(FROM_PHONE, session),
            woken: false,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pairing {
    pub secret: [u8; 32],
    /// Highest WAKE counter accepted so far.
    pub last_counter: u64,
    pub pc: Option<IpAddr>,
}

pub enum Event {
    Send(Vec<u8>, SocketAddr),
    /// HELLO confirmed (or WAKE accepted): send audio to this peer.
    Active(Arc<Peer>),
    /// The active peer sent STOP.
    Stopped,
    /// A WAKE was accepted; the engine should leave standby.
    Woken(Arc<Peer>),
    /// The PC handed over a (new) pairing secret, or the counter advanced.
    PairingChanged(Pairing),
}

pub struct PhoneControl {
    peer: Option<Arc<Peer>>,
    pending: Option<(Arc<Peer>, u64)>,
    last_hello: u64,
    last_exchange: Option<u64>,
    pub pairing: Option<Pairing>,
    pairing_key: Option<ControlKey>,
    /// In standby only authenticated WAKE datagrams are accepted.
    pub standby: bool,
}

impl PhoneControl {
    pub fn new(pairing: Option<Pairing>) -> Self {
        let pairing_key = pairing.as_ref().map(|p| ControlKey::new(&p.secret));
        PhoneControl { peer: None, pending: None, last_hello: 0, last_exchange: None, pairing, pairing_key, standby: false }
    }

    pub fn set_pairing(&mut self, pairing: Option<Pairing>) {
        self.pairing_key = pairing.as_ref().map(|p| ControlKey::new(&p.secret));
        self.pairing = pairing;
    }

    /// The peer that currently receives audio, if its lease is valid.
    pub fn active(&self, now: u64) -> Option<&Arc<Peer>> {
        self.peer.as_ref().filter(|_| now.saturating_sub(self.last_hello) < LEASE_MS)
    }

    pub fn last_hello(&self) -> u64 {
        self.last_hello
    }

    /// Forget the current peer (e.g. returning to standby).
    pub fn drop_peer(&mut self) {
        self.peer = None;
        self.pending = None;
        self.last_hello = 0;
    }

    pub fn on_datagram(&mut self, data: &[u8], from: SocketAddr, now: u64, events: &mut Vec<Event>) {
        if keyx::is_offer(data, FROM_PC) {
            if !self.standby {
                self.on_offer(data, from, now, events);
            }
            return;
        }
        let Some(header) = wire::parse_header(data) else { return };
        if header.kind == WAKE {
            self.on_wake(data, from, now, events);
            return;
        }
        let lease_valid = now.saturating_sub(self.last_hello) < LEASE_MS;
        let current = self.peer.clone().filter(|p| p.addr == from);
        let mut candidate = current.clone();
        let mut opened = candidate.as_ref().and_then(|p| p.key.control().open(data).map(|(h, pl)| (h, pl.to_vec())));
        if opened.is_none() {
            if let Some((pending, until)) = self.pending.clone() {
                if now < until && pending.addr == from && (self.peer.is_none() || !lease_valid) {
                    opened = pending.key.control().open(data).map(|(h, pl)| (h, pl.to_vec()));
                    candidate = Some(pending);
                }
            }
        }
        let (Some((h, payload)), Some(peer)) = (opened, candidate) else { return };
        if h.session != peer.session {
            return;
        }
        match h.kind {
            HELLO => {
                self.last_hello = now;
                let is_pending = self.pending.as_ref().is_some_and(|(p, _)| Arc::ptr_eq(p, &peer));
                if is_pending {
                    self.pending = None;
                }
                self.peer = Some(peer.clone());
                events.push(Event::Active(peer));
            }
            STOP => {
                if current.as_ref().is_some_and(|c| Arc::ptr_eq(c, &peer)) {
                    self.peer = None;
                    self.last_hello = 0;
                    events.push(Event::Stopped);
                }
            }
            PAIR if payload.len() == 32 => {
                let masked: [u8; 32] = payload[..].try_into().unwrap();
                let secret = wire::pair_payload(&peer.key, h.session, h.counter, &masked);
                let last = self.pairing.as_ref().filter(|p| p.secret == secret).map_or(0, |p| p.last_counter);
                let pairing = Pairing { secret, last_counter: last.max(h.counter), pc: Some(from.ip()) };
                if self.pairing.as_ref() != Some(&pairing) {
                    self.set_pairing(Some(pairing.clone()));
                    events.push(Event::PairingChanged(pairing));
                }
                // Acknowledge so the PC knows the pairing is stored.
                let ack = peer.key.control().seal(PAIR, peer.session, h.counter, &[]);
                events.push(Event::Send(ack, from));
            }
            _ => {}
        }
    }

    fn on_offer(&mut self, data: &[u8], from: SocketAddr, now: u64, events: &mut Vec<Event>) {
        let reply = if let Some(cur) = self.peer.clone().filter(|p| p.addr == from && p.request[..] == data[..]) {
            cur
        } else {
            if self.peer.is_some() && now.saturating_sub(self.last_hello) < LEASE_MS {
                return; // never take over an active connection
            }
            match self.pending.clone() {
                Some((p, until)) if now < until && p.addr == from && p.request[..] == data[..] => p,
                _ => {
                    if self.last_exchange.is_some_and(|t| now.saturating_sub(t) < EXCHANGE_INTERVAL_MS) {
                        return;
                    }
                    self.last_exchange = Some(now);
                    let Some(p) = Peer::new(from, data) else { return };
                    let p = Arc::new(p);
                    self.pending = Some((p.clone(), now + LEASE_MS));
                    p
                }
            }
        };
        events.push(Event::Send(reply.reply.to_vec(), from));
    }

    fn on_wake(&mut self, data: &[u8], from: SocketAddr, now: u64, events: &mut Vec<Event>) {
        let (Some(key), Some(pairing)) = (self.pairing_key.as_ref(), self.pairing.as_mut()) else { return };
        let Some((h, offer)) = key.open(data) else { return };
        if !keyx::is_offer(offer, FROM_PC) || keyx::offer_session(offer) != h.session {
            return;
        }
        // Retries of the same wake-up re-send the same reply.
        if let Some(cur) = self.peer.clone().filter(|p| p.addr == from && p.request[..] == offer[..]) {
            events.push(Event::Send(cur.reply.to_vec(), from));
            return;
        }
        if h.counter <= pairing.last_counter {
            return; // replay
        }
        let active_elsewhere = self.peer.as_ref().is_some_and(|p| p.addr.ip() != from.ip())
            && now.saturating_sub(self.last_hello) < LEASE_MS;
        if active_elsewhere {
            return;
        }
        let Some(mut peer) = Peer::new(from, offer) else { return };
        pairing.last_counter = h.counter;
        pairing.pc = Some(from.ip());
        let pairing = pairing.clone();
        peer.woken = true;
        let peer = Arc::new(peer);
        self.pending = None;
        self.peer = Some(peer.clone());
        self.last_hello = now;
        self.standby = false;
        events.push(Event::Send(peer.reply.to_vec(), from));
        events.push(Event::PairingChanged(pairing));
        events.push(Event::Woken(peer.clone()));
        events.push(Event::Active(peer));
    }
}

/// ANNOUNCE payload: the phone's UDP port (LE u16).
pub fn announce(pairing: &Pairing, counter: u64, port: u16) -> Vec<u8> {
    ControlKey::new(&pairing.secret).seal(wire::ANNOUNCE, 0, counter, &port.to_le_bytes())
}
