use std::net::SocketAddr;

use toaster_core::keyx::{self, KeyExchange, FROM_PC, FROM_PHONE};
use toaster_core::phone::{Event, Pairing, PhoneControl};
use toaster_core::wire::{self, HELLO, PAIR, STOP, WAKE};
use toaster_core::{ControlKey, SessionKey};

fn addr(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

fn replies(ev: &[Event]) -> Vec<Vec<u8>> {
    ev.iter().filter_map(|e| if let Event::Send(d, _) = e { Some(d.clone()) } else { None }).collect()
}

/// PC side of a manual connection: offer, derive, HELLO.
fn connect(ctl: &mut PhoneControl, from: SocketAddr, session: u64, now: u64) -> SessionKey {
    let pc = KeyExchange::new();
    let mut ev = Vec::new();
    ctl.on_datagram(&pc.offer(FROM_PC, session), from, now, &mut ev);
    let reply = replies(&ev).pop().expect("reply");
    assert!(keyx::is_offer(&reply, FROM_PHONE));
    let key = SessionKey::new(&pc.derive(&reply).unwrap());
    ev.clear();
    ctl.on_datagram(&key.control().seal(HELLO, session, 0, &[]), from, now + 10, &mut ev);
    assert!(ev.iter().any(|e| matches!(e, Event::Active(_))));
    key
}

#[test]
fn manual_handshake_lease_and_takeover_protection() {
    let mut ctl = PhoneControl::new(None);
    let pc = addr("192.168.1.10:50000");
    let key = connect(&mut ctl, pc, 42, 1000);
    assert!(ctl.active(2000).is_some() && ctl.active(1010 + 3500).is_none());

    let mut ev = Vec::new();
    ctl.on_datagram(&KeyExchange::new().offer(FROM_PC, 7), addr("192.168.1.11:5"), 1500, &mut ev);
    assert!(ev.is_empty(), "active connection is not taken over");
    ctl.on_datagram(&key.control().seal(HELLO, 42, 0, &[]), addr("192.168.1.11:5"), 1500, &mut ev);
    assert!(ev.is_empty(), "HELLO from another endpoint ignored");
    ctl.on_datagram(&key.control().seal(STOP, 42, 0, &[]), pc, 1600, &mut ev);
    assert!(matches!(ev.as_slice(), [Event::Stopped]));
    assert!(ctl.active(1700).is_none());
}

#[test]
fn identical_offer_gets_identical_reply_and_rate_limit() {
    let mut ctl = PhoneControl::new(None);
    let pc = KeyExchange::new();
    let offer = pc.offer(FROM_PC, 9);
    let from = addr("10.0.0.2:4000");
    let mut ev = Vec::new();
    ctl.on_datagram(&offer, from, 0, &mut ev);
    ctl.on_datagram(&offer, from, 100, &mut ev);
    let r = replies(&ev);
    assert_eq!(r.len(), 2);
    assert_eq!(r[0], r[1], "retry gets the same reply");
    ev.clear();
    ctl.on_datagram(&KeyExchange::new().offer(FROM_PC, 10), from, 150, &mut ev);
    assert!(ev.is_empty(), "at most four new key pairs per second");
    ctl.on_datagram(&KeyExchange::new().offer(FROM_PC, 10), from, 400, &mut ev);
    assert_eq!(replies(&ev).len(), 1);
}

#[test]
fn pairing_wake_replay_and_standby() {
    let mut ctl = PhoneControl::new(None);
    let pc = addr("192.168.1.10:50000");
    let key = connect(&mut ctl, pc, 42, 1000);
    let secret = [3u8; 32];
    let masked = wire::pair_payload(&key, 42, 500, &secret);
    let mut ev = Vec::new();
    ctl.on_datagram(&key.control().seal(PAIR, 42, 500, &masked), pc, 1100, &mut ev);
    let pairing = ev.iter().find_map(|e| if let Event::PairingChanged(p) = e { Some(p.clone()) } else { None }).unwrap();
    assert_eq!(pairing, Pairing { secret, last_counter: 500, pc: Some(pc.ip()) });
    let ack = replies(&ev).pop().unwrap();
    assert_eq!(key.control().open(&ack).unwrap().0.kind, PAIR, "pairing acknowledged");

    // PC disconnects, phone goes to standby.
    ev.clear();
    ctl.on_datagram(&key.control().seal(STOP, 42, 0, &[]), pc, 1200, &mut ev);
    ctl.drop_peer();
    ctl.standby = true;
    ev.clear();
    ctl.on_datagram(&KeyExchange::new().offer(FROM_PC, 1), addr("192.168.1.66:1"), 9000, &mut ev);
    assert!(ev.is_empty(), "standby ignores unpaired offers");

    let pk = ControlKey::new(&secret);
    let forged = ControlKey::new(&[4u8; 32]).seal(WAKE, 77, 501, &KeyExchange::new().offer(FROM_PC, 77));
    ctl.on_datagram(&forged, pc, 9000, &mut ev);
    assert!(ev.is_empty(), "WAKE with a wrong secret ignored");

    let exchange = KeyExchange::new();
    let wake = pk.seal(WAKE, 77, 501, &exchange.offer(FROM_PC, 77));
    ctl.on_datagram(&wake, pc, 9000, &mut ev);
    assert!(ev.iter().any(|e| matches!(e, Event::Woken(_))), "paired WAKE wakes the phone");
    assert!(ctl.active(9001).is_some(), "audio starts without waiting for HELLO");
    assert!(!ctl.standby);
    let reply = replies(&ev).pop().unwrap();
    let session_key = SessionKey::new(&exchange.derive(&reply).unwrap());
    ev.clear();
    ctl.on_datagram(&wake, pc, 9150, &mut ev);
    assert_eq!(replies(&ev), vec![reply], "WAKE retry gets the same reply");
    assert!(!ev.iter().any(|e| matches!(e, Event::Woken(_))));
    ev.clear();
    ctl.on_datagram(&session_key.control().seal(HELLO, 77, 0, &[]), pc, 9200, &mut ev);
    assert!(ev.iter().any(|e| matches!(e, Event::Active(_))), "HELLO continues the woken session");

    ctl.on_datagram(&session_key.control().seal(STOP, 77, 0, &[]), pc, 9300, &mut ev);
    ctl.drop_peer();
    ctl.standby = true;
    ev.clear();
    ctl.on_datagram(&wake, pc, 20000, &mut ev);
    assert!(ev.is_empty(), "replayed WAKE ignored");
    let older = pk.seal(WAKE, 78, 400, &KeyExchange::new().offer(FROM_PC, 78));
    ctl.on_datagram(&older, pc, 20000, &mut ev);
    assert!(ev.is_empty(), "older counter ignored");
}
