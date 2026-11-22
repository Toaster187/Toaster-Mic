//! Port of the 0.4 C#/Java protocol and audio checks, extended for protocol v3.

use std::net::UdpSocket;
use std::time::Duration;

use sha2::{Digest, Sha256};
use toaster_core::calibration::{distinct, identify};
use toaster_core::keyx::{self, KeyExchange, FROM_PC, FROM_PHONE};
use toaster_core::wire::{self, flags, Header, AUDIO, HELLO, STOP, WAKE};
use toaster_core::{AudioPacket, ConnectionLease, ControlKey, JitterBuffer, Playout, SessionKey};

fn key() -> [u8; 32] {
    Sha256::digest(b"interop-test-key").into()
}

fn pcm(l: i16, r: i16, channels: u8) -> Vec<i16> {
    let frames = wire::frames_for(channels) as usize;
    let mut s = Vec::with_capacity(frames * channels as usize);
    for _ in 0..frames {
        for c in 0..channels {
            s.push(match c {
                0 => l,
                1 => r,
                _ => l / 2,
            });
        }
    }
    s
}

fn p(seq: u32, l: i16, r: i16) -> AudioPacket {
    AudioPacket { sequence: seq, channels: 2, flags: 0, samples: pcm(l, r, 2) }
}
fn p1(seq: u32) -> AudioPacket {
    p(seq, 1000, -2000)
}
fn pm(seq: u32, v: i16) -> AudioPacket {
    AudioPacket { sequence: seq, channels: 1, flags: 0, samples: pcm(v, v, 1) }
}

fn read(o: &mut Playout) -> Vec<i16> {
    let mut b = vec![0i16; 480];
    o.read(&mut b);
    b
}

fn sealed(channels: u8, seq: u32, k: &SessionKey) -> Vec<u8> {
    let mut buf = vec![0u8; wire::audio_len(channels)];
    let samples = pcm(1000, -2000, channels);
    wire::write_pcm(&samples, &mut buf[wire::HEADER..]);
    let n = k.seal_audio(&mut buf, channels, 0, 42, seq);
    buf.truncate(n);
    buf
}

#[test]
fn encrypted_stereo_roundtrip_and_tamper() {
    let k = SessionKey::new(&key());
    let encoded = sealed(2, 7, &k);
    assert_eq!(encoded.len(), 1012, "stereo fits a normal WLAN MTU without fragmentation");
    let plain = pcm(1000, -2000, 2);
    let mut bytes = vec![0u8; plain.len() * 2];
    wire::write_pcm(&plain, &mut bytes);
    assert_ne!(&encoded[wire::HEADER..wire::HEADER + bytes.len()], &bytes[..], "audio is encrypted on the wire");

    let mut copy = encoded.clone();
    let h = k.open_audio(&mut copy).expect("roundtrip");
    assert_eq!((h.sequence, h.session, h.channels), (7, 42, 2));
    assert_eq!(&copy[wire::HEADER..copy.len() - wire::TAG], &bytes[..]);

    for i in 0..encoded.len() {
        let mut t = encoded.clone();
        t[i] ^= 1;
        assert!(k.open_audio(&mut t).is_none(), "tamper accepted at {i}");
    }
    let other = SessionKey::new(&[0u8; 32]);
    assert!(other.open_audio(&mut encoded.clone()).is_none(), "different session key rejected");
    for n in 0..encoded.len() {
        assert!(k.open_audio(&mut encoded[..n].to_vec()).is_none(), "truncation accepted");
    }
    let mut longer = encoded.clone();
    longer.push(0);
    assert!(k.open_audio(&mut longer).is_none(), "trailing bytes rejected");
}

#[test]
fn multichannel_packets_never_fragment() {
    let k = SessionKey::new(&key());
    for ch in 1..=wire::MAX_CHANNELS {
        let len = wire::audio_len(ch);
        assert!(len <= wire::MAX_DATAGRAM, "{ch} channels: {len} bytes");
        let mut e = sealed(ch, 1, &k);
        let h = k.open_audio(&mut e).unwrap();
        assert_eq!(h.frames, wire::frames_for(ch));
    }
    assert_eq!(wire::audio_len(3), 1012);
    assert!(wire::parse_header(&vec![0u8; wire::audio_len(3)]).is_none());
}

#[test]
fn authenticated_control_packets() {
    let k = SessionKey::new(&key());
    for kind in [HELLO, STOP] {
        let d = k.control().seal(kind, 42, 0, &[]);
        assert_eq!(d.len(), wire::HEADER + wire::TAG);
        let (h, payload) = k.control().open(&d).unwrap();
        assert_eq!((h.kind, h.session, payload.len()), (kind, 42, 0));
        for i in 0..d.len() {
            let mut t = d.clone();
            t[i] ^= 0x80;
            assert!(k.control().open(&t).is_none());
        }
    }
    assert!(SessionKey::new(&[1; 32]).control().open(&k.control().seal(HELLO, 42, 0, &[])).is_none());
    // Control and audio cannot be confused.
    assert!(k.control().open(&sealed(2, 1, &k)).is_none());
    assert!(k.open_audio(&mut k.control().seal(HELLO, 42, 0, &[])).is_none());
}

#[test]
fn wake_carries_offer_and_counter() {
    let secret = [7u8; 32];
    let pairing = ControlKey::new(&secret);
    let pc = KeyExchange::new();
    let offer = pc.offer(FROM_PC, 99);
    let d = pairing.seal(WAKE, 99, 1234, &offer);
    let (h, payload) = pairing.open(&d).unwrap();
    assert_eq!((h.kind, h.counter, h.session), (WAKE, 1234, 99));
    assert!(keyx::is_offer(payload, FROM_PC));
    assert!(ControlKey::new(&[8u8; 32]).open(&d).is_none(), "wrong pairing secret rejected");
    let mut replay = d.clone();
    replay[28] ^= 1; // a forged counter breaks the tag
    assert!(pairing.open(&replay).is_none());
}

#[test]
fn pair_payload_is_masked_and_recoverable() {
    let k = SessionKey::new(&key());
    let secret = [0x5Au8; 32];
    let masked = wire::pair_payload(&k, 42, 3, &secret);
    assert_ne!(masked, secret);
    assert_eq!(wire::pair_payload(&k, 42, 3, &masked), secret);
    assert_ne!(wire::pair_payload(&k, 42, 4, &secret), masked);
}

#[test]
fn malformed_datagrams_handled() {
    let k = SessionKey::new(&key());
    let mut state = 1u64;
    for _ in 0..10_000 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let len = (state % 1600) as usize;
        let mut noise: Vec<u8> = (0..len).map(|i| (state >> (i % 56)) as u8).collect();
        if len > 8 && state % 3 == 0 {
            noise[..5].copy_from_slice(b"TMIC\x03");
        }
        let _ = wire::parse_header(&noise);
        let _ = k.open_audio(&mut noise.clone());
        let _ = k.control().open(&noise);
    }
    let mut v2 = sealed(2, 1, &k);
    v2[4] = 2;
    assert!(wire::parse_header(&v2).is_none(), "protocol v2 rejected");
}

#[test]
fn code_free_handshake() {
    let pc = KeyExchange::new();
    let phone = KeyExchange::new();
    let request = pc.offer(FROM_PC, 42);
    let reply = phone.offer(FROM_PHONE, 42);
    assert!(keyx::is_offer(&request, FROM_PC) && keyx::is_offer(&reply, FROM_PHONE));
    assert_eq!(keyx::offer_session(&reply), 42);
    assert_eq!(pc.derive(&reply).unwrap(), phone.derive(&request).unwrap(), "P-256 agreement");
    assert!(!keyx::is_offer(&reply, FROM_PC), "wrong direction rejected");
    for n in 0..80 {
        assert!(!keyx::is_offer(&reply[..n], FROM_PHONE));
    }
    let mut long = reply.to_vec();
    long.push(0);
    assert!(!keyx::is_offer(&long, FROM_PHONE));
    let mut invalid = reply;
    invalid[16..].fill(0);
    assert!(pc.derive(&invalid).is_none(), "invalid curve point rejected");
    let restarted = KeyExchange::new();
    assert_ne!(pc.derive(&reply), pc.derive(&restarted.offer(FROM_PHONE, 42)), "phone restart gives a fresh key");
    assert_eq!(pc.derive(&reply), pc.derive(&reply), "retried handshake derives the same key");
}

#[test]
fn udp_loopback_handshake_and_audio() {
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    let client = UdpSocket::bind("127.0.0.1:0").unwrap();
    server.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    client.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    client.connect(server.local_addr().unwrap()).unwrap();
    let pc = KeyExchange::new();
    let phone = KeyExchange::new();
    let mut buf = [0u8; 2048];
    client.send(&pc.offer(FROM_PC, 42)).unwrap();
    let (n, from) = server.recv_from(&mut buf).unwrap();
    assert!(keyx::is_offer(&buf[..n], FROM_PC));
    let phone_key = phone.derive(&buf[..n]).unwrap();
    server.send_to(&phone.offer(FROM_PHONE, 42), from).unwrap();
    let n = client.recv(&mut buf).unwrap();
    let pc_key = pc.derive(&buf[..n]).unwrap();
    assert_eq!(pc_key, phone_key);
    let pk = SessionKey::new(&pc_key);
    client.send(&pk.control().seal(HELLO, 42, 0, &[])).unwrap();
    let n = server.recv(&mut buf).unwrap();
    let phk = SessionKey::new(&phone_key);
    assert_eq!(phk.control().open(&buf[..n]).unwrap().0.session, 42);
    server.send_to(&sealed(3, 7, &phk), from).unwrap();
    let n = client.recv(&mut buf).unwrap();
    let h = pk.open_audio(&mut buf[..n]).unwrap();
    assert_eq!((h.kind, h.channels, h.sequence), (AUDIO, 3, 7));
}

#[test]
fn jitter_reorders_conceals_and_discards_late() {
    let mut o = Playout::new(15, false, None);
    o.jitter.push(p(10, 1000, -2000));
    o.jitter.push(p(12, 1200, -2000));
    assert!(read(&mut o).iter().all(|&x| x == 0), "waits for startup buffer");
    o.jitter.push(p(11, 1100, -2000));
    assert_eq!([read(&mut o)[0], read(&mut o)[0], read(&mut o)[0]], [1000, 1100, 1200], "out-of-order restored");

    let mut lost = Playout::new(5, false, None);
    lost.jitter.push(p1(1));
    lost.jitter.push(p(3, 3000, 0));
    read(&mut lost);
    assert!(read(&mut lost).iter().all(|&x| x == 0));
    assert_eq!(read(&mut lost)[0], 3000);
    assert_eq!(lost.jitter.stats().lost, 1, "packet loss concealed without blocking future audio");
    lost.jitter.push(p1(2));
    assert_eq!(lost.jitter.stats().late, 1, "late packets discarded");
}

#[test]
fn mono_stereo_gain_and_mute() {
    let mut mono = Playout::new(5, false, None);
    mono.jitter.push(pm(1, 2345));
    let b = read(&mut mono);
    assert_eq!((b[0], b[1]), (2345, 2345), "mono duplicated to both outputs");

    let mut stereo = Playout::new(5, false, None);
    stereo.jitter.push(p1(1));
    let b = read(&mut stereo);
    assert_eq!((b[0], b[1]), (1000, -2000), "left and right stay independent");

    let mut v = Playout::new(5, false, None);
    v.gain = 2.0;
    v.jitter.push(p(1, 25000, -25000));
    let b = read(&mut v);
    assert_eq!((b[0], b[1]), (32767, -32768), "gain clips safely");
    v.muted = true;
    v.jitter.push(p1(2));
    assert!(read(&mut v).iter().all(|&x| x == 0), "mute emits silence");
    v.muted = false;
    v.jitter.push(p(3, 1234, 0));
    assert_eq!(read(&mut v)[0], 2468, "unmute resumes current audio");

    let mut boost = Playout::new(5, false, None);
    boost.gain = 10.0;
    boost.jitter.push(p(1, 1000, -1000));
    let b = read(&mut boost);
    assert_eq!((b[0], b[1]), (10000, -10000), "1000 % is a real 20 dB boost");
}

#[test]
fn adaptive_reserve_follows_device_and_underruns() {
    let mut a = Playout::new(5, true, None);
    let mut period = vec![0i16; 960];
    a.jitter.push(p1(0));
    a.read(&mut period);
    assert!(period.iter().all(|&x| x == 0) && a.jitter.target_ms() == 15, "reserve covers a 10 ms device callback");
    a.jitter.push(p1(1));
    a.jitter.push(p1(2));
    a.read(&mut period);
    assert_eq!(period[0], 1000);
    assert_eq!(a.jitter.stats().lost, 0, "device callback plays without built-in underrun");
    read(&mut a);
    let fade = read(&mut a);
    assert!(fade[0] > 0 && fade[478] == 0 && a.jitter.target_ms() == 20, "underrun fades out and raises the reserve");
    for _ in 0..6 {
        read(&mut a);
    }
    for i in 3..7 {
        a.jitter.push(p(i, 2000, 0));
    }
    let recovered = read(&mut a);
    assert!(recovered[100] == 2000 && a.jitter.stats().late == 0, "late burst recovers without chasing the sequence");
    a.reset();
    assert!(a.jitter.target_ms() == 5 && read(&mut a).iter().all(|&x| x == 0), "new session clears learned reserve");
}

#[test]
fn batched_capture_and_stalls_stay_bounded() {
    let mut c = Playout::new(5, true, None);
    let mut period = vec![0i16; 960];
    let mut sent = 0u32;
    for _ in 0..1000 {
        c.jitter.push(p1(sent));
        c.jitter.push(p1(sent + 1));
        sent += 2;
        c.read(&mut period);
    }
    assert!(c.jitter.stats().lost == 0 && c.jitter.target_ms() == 15, "batched capture does not stutter");
    for _ in 0..100 {
        for _ in 0..14 {
            read(&mut c);
        }
        for _ in 0..12 {
            c.jitter.push(p1(sent));
            sent += 1;
        }
    }
    assert!(c.jitter.target_ms() <= 60 && c.jitter.stats().buffered_ms <= 95, "stalls keep latency bounded");
}

#[test]
fn speech_pause_trimming() {
    let mut period = vec![0i16; 960];
    let mut q = Playout::new(5, true, None);
    let mut s = 0u32;
    q.read(&mut vec![0i16; 4800]);
    for _ in 0..11 {
        q.jitter.push(p(s, 0, 0));
        s += 1;
    }
    for _ in 0..1200 {
        q.jitter.push(p(s, 0, 0));
        q.jitter.push(p(s + 1, 0, 0));
        s += 2;
        q.read(&mut period);
    }
    assert!(q.jitter.target_ms() == 15 && q.jitter.stats().buffered_ms <= 15 && q.jitter.trimmed_packets() > 0, "pauses reduce reserve");

    let mut sp = Playout::new(5, true, None);
    sp.muted = true;
    let mut s = 0u32;
    sp.read(&mut vec![0i16; 4800]);
    for _ in 0..11 {
        sp.jitter.push(p1(s));
        s += 1;
    }
    for _ in 0..1200 {
        sp.jitter.push(p1(s));
        sp.jitter.push(p1(s + 1));
        s += 2;
        sp.read(&mut period);
    }
    assert!(sp.jitter.target_ms() == 55 && sp.jitter.trimmed_packets() == 0, "speech never trimmed, even when muted");

    let mut pr = Playout::new(5, true, None);
    let mut s = 0u32;
    for _ in 0..3 {
        pr.jitter.push(p(s, 0, 0));
        s += 1;
    }
    for _ in 0..200 {
        pr.jitter.push(p(s, 0, 0));
        pr.jitter.push(p(s + 1, 0, 0));
        s += 2;
        pr.read(&mut period);
    }
    for _ in 0..5 {
        pr.jitter.push(p(s, 2345, -2345));
        s += 1;
    }
    let audible = (0..10).filter(|_| read(&mut pr)[100] == 2345).count();
    assert_eq!(audible, 5, "quiet-prefix trim preserves queued speech");

    let mut bq = Playout::new(5, true, None);
    bq.gain = 10.0;
    let mut s = 0u32;
    bq.read(&mut vec![0i16; 4800]);
    for _ in 0..11 {
        bq.jitter.push(p(s, 50, -50));
        s += 1;
    }
    for _ in 0..600 {
        bq.jitter.push(p(s, 50, -50));
        bq.jitter.push(p(s + 1, 50, -50));
        s += 2;
        bq.read(&mut period);
    }
    assert_eq!(bq.jitter.trimmed_packets(), 0, "amplified low-level speech is not silence");

    let mut sel = Playout::new(60, true, None);
    let mut s = 0u32;
    for _ in 0..12 {
        sel.jitter.push(p(s, 0, 0));
        s += 1;
    }
    for _ in 0..600 {
        sel.jitter.push(p(s, 0, 0));
        sel.jitter.push(p(s + 1, 0, 0));
        s += 2;
        sel.read(&mut period);
    }
    assert_eq!(sel.jitter.target_ms(), 60, "trimming respects the configured minimum reserve");
}

#[test]
fn bounds_wrap_dropout_duplicates_and_partial_reads() {
    let mut b = JitterBuffer::new(15, false);
    for i in 0..1000 {
        b.push(p1(i));
    }
    assert!(b.stats().buffered_ms <= 65 && b.stats().resyncs > 0, "slow consumer cannot grow latency");

    let mut w = Playout::new(5, false, None);
    w.jitter.push(p1(u32::MAX));
    w.jitter.push(p(0, 2000, 0));
    assert_eq!([read(&mut w)[0], read(&mut w)[0]], [1000, 2000], "sequence wrap");

    let mut d = Playout::new(5, false, None);
    d.jitter.push(p1(0));
    read(&mut d);
    for _ in 0..20 {
        read(&mut d);
    }
    d.jitter.push(p(500, 5000, 0));
    assert_eq!(read(&mut d)[0], 5000, "restarts after prolonged dropout");

    let mut dup = JitterBuffer::new(5, false);
    dup.push(p1(1));
    dup.push(p1(1));
    assert_eq!(dup.stats().received, 1, "duplicate ignored");

    let mut part = Playout::new(5, false, None);
    part.jitter.push(p1(1));
    let mut half = vec![0i16; 240];
    part.read(&mut half);
    part.read(&mut half);
    assert_eq!(half[0], 1000, "device reads may split a packet");
    part.reset();
    assert!(read(&mut part).iter().all(|&x| x == 0), "rekey clears buffered audio");
    part.jitter.push(p(0, 321, 0));
    assert_eq!(read(&mut part)[0], 321, "new recording can restart the sequence");
}

#[test]
fn three_channel_packets_play_and_report_ms() {
    let mut o = Playout::new(5, true, None);
    let pkt = |seq| AudioPacket { sequence: seq, channels: 3, flags: 0, samples: pcm(1500, -1500, 3) };
    o.jitter.push(pkt(0));
    let mut out = vec![0i16; 320];
    o.read(&mut out);
    assert_eq!(o.jitter.packet_frames(), 160);
    assert_eq!(o.jitter.target_ms(), 7, "two 3.33 ms packets reserve a 160-frame read");
    assert!(out.iter().all(|&x| x == 0));
    o.jitter.push(pkt(1));
    o.read(&mut out);
    assert_eq!((out[0], out[1]), (1500, -1500), "without studio processor the first two channels pass");
}

#[test]
fn concurrent_producer_and_consumer() {
    use std::sync::{Arc, Mutex};
    let o = Arc::new(Mutex::new(Playout::new(15, false, None)));
    let prod = {
        let o = o.clone();
        std::thread::spawn(move || {
            for i in 0..10_000 {
                o.lock().unwrap().jitter.push(p1(i));
            }
        })
    };
    let mut buf = vec![0i16; 480];
    for _ in 0..10_000 {
        o.lock().unwrap().read(&mut buf);
    }
    prod.join().unwrap();
    assert!(o.lock().unwrap().jitter.stats().buffered_ms <= 65);
}

#[test]
fn lease_expires_at_deadline() {
    let lease = ConnectionLease::new(Duration::from_millis(100));
    assert!(!lease.is_active_at(10));
    lease.observe_at(50);
    assert!(lease.is_active_at(149) && !lease.is_active_at(150));
    lease.clear();
    assert!(!lease.is_active_at(60));
}

#[test]
fn calibration_dominance_ambiguity_and_mappings() {
    let quiet = [10.0, 10.0, 10.0];
    for c in 0..3 {
        let mut touch = [30.0, 30.0, 30.0];
        touch[c] = 500.0;
        assert_eq!(identify(&quiet, &touch), Some(c));
    }
    assert_eq!(identify(&quiet, &[500.0, 450.0, 30.0]), None);
    assert_eq!(identify(&quiet, &quiet), None);
    assert_eq!(identify(&quiet, &[f64::NAN, 900.0, 20.0]), None);
    assert_eq!(identify(&[500.0, 10.0, 10.0], &[510.0, 100.0, 20.0]), None);
    for t in -1..=3 {
        for b in -1..=3 {
            for r in -1..=3 {
                let expect = (0..3).contains(&t) && (0..3).contains(&b) && (0..3).contains(&r) && t != b && t != r && b != r;
                assert_eq!(distinct(&[t, b, r], 3), expect);
            }
        }
    }
    assert!(distinct(&[1, 0], 2) && !distinct(&[1, 1], 2));
}

#[test]
fn header_roundtrip_and_flags() {
    let h = Header::audio(3, flags::STUDIO | flags::FAST_START | toaster_core::Profile::Battery.bits(), 9, 77);
    let mut b = vec![0u8; h.datagram_len()];
    h.write(&mut b);
    let parsed = wire::parse_header(&b).unwrap();
    assert_eq!(parsed, h);
    assert_eq!(toaster_core::Profile::from_flags(parsed.flags), toaster_core::Profile::Battery);
}
