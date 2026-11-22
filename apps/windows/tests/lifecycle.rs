//! Lifecycle of the real PC receiver against the simulated phone (same
//! control logic as the Android engine). The audio device is simulated.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use toaster_core::phone::Pairing;
use toaster_core::sim::{SimConfig, SimPhone};
use toaster_core::Playout;
use toaster_windows::receiver::{Config, OutputFactory, Receiver, WakeConfig};

struct FakeDevice {
    stop: Arc<AtomicBool>,
    release: Option<Arc<AtomicBool>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for FakeDevice {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Some(r) = &self.release {
            // A slow driver: shutting down takes until the test releases it.
            let deadline = Instant::now() + Duration::from_secs(5);
            while !r.load(Ordering::Acquire) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

fn device(release: Option<Arc<AtomicBool>>) -> OutputFactory {
    Box::new(move |playout: Arc<Mutex<Playout>>, _fault| {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut buf = vec![0i16; 960];
            while !s.load(Ordering::Acquire) {
                playout.lock().unwrap().read(&mut buf);
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        Ok(Box::new(FakeDevice { stop, release, thread: Some(thread) }) as Box<dyn Send>)
    })
}

fn config(phone: SocketAddr) -> Config {
    Config {
        host: Ipv4Addr::LOCALHOST,
        port: phone.port(),
        target_ms: 15,
        gain: 1.0,
        muted: false,
        wake: None,
        pair: None,
        studio: Some(Box::new(toaster_dsp::Studio::new(toaster_dsp::NoiseLevel::Medium))),
    }
}

fn wait(timeout: Duration, mut ok: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    ok()
}

#[test]
fn handshake_audio_nonblocking_disconnect_and_stop() {
    let phone = SimPhone::start(SimConfig { channels: 2, ..Default::default() }).unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let r = Receiver::start(config(phone.addr), device(Some(release.clone()))).unwrap();
    assert!(wait(Duration::from_secs(5), || r.connected()), "receiver completes encrypted handshake and gets audio");
    assert!(wait(Duration::from_secs(2), || r.snapshot().stats.received > 20));
    assert_eq!(r.channels(), 2);
    assert!(r.status().contains("Stereo"), "{}", r.status());

    let t = Instant::now();
    let done = r.shutdown();
    assert!(t.elapsed() < Duration::from_millis(100), "disconnect does not wait for a slow audio driver");
    assert!(!done.is_finished());
    release.store(true, Ordering::Release);
    assert!(wait(Duration::from_secs(3), || done.is_finished()), "output released exactly once");
    assert!(wait(Duration::from_secs(2), || phone.state.stopped.load(Ordering::Relaxed) >= 1), "disconnect sends STOP");
}

#[test]
fn rapid_connect_disconnect_cycles() {
    let phone = SimPhone::start(SimConfig::default()).unwrap();
    for _ in 0..20 {
        let r = Receiver::start(config(phone.addr), device(None)).unwrap();
        let _ = r.shutdown().join();
    }
    let r = Receiver::start(config(phone.addr), device(None)).unwrap();
    assert!(wait(Duration::from_secs(6), || r.connected()), "a new connection works after 20 cycles");
    let _ = r.shutdown().join();
}

#[test]
fn pairing_then_wake_from_standby() {
    let secret = [9u8; 32];
    let phone = SimPhone::start(SimConfig { channels: 3, studio: true, always_on: true, fast_start_ms: 300, ..Default::default() }).unwrap();
    let mut c = config(phone.addr);
    c.pair = Some((secret, 100));
    let r = Receiver::start(c, device(None)).unwrap();
    assert!(wait(Duration::from_secs(5), || r.paired()), "phone acknowledges the pairing");
    let stored = phone.state.pairing.lock().unwrap().clone().unwrap();
    assert_eq!(stored.secret, secret);
    let _ = r.shutdown().join();
    assert!(wait(Duration::from_secs(5), || phone.state.standby.load(Ordering::Acquire)), "phone returns to standby");

    // Plain offers are ignored in standby.
    let r = Receiver::start(config(phone.addr), device(None)).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!r.connected(), "standby phone ignores unpaired offers");
    let _ = r.shutdown().join();

    for counter in [200u64, 2000] {
        let mut c = config(phone.addr);
        c.wake = Some(WakeConfig { secret, counter });
        let started = Instant::now();
        let r = Receiver::start(c, device(None)).unwrap();
        assert!(wait(Duration::from_secs(3), || r.connected()), "paired WAKE starts audio");
        let first = r.first_audio_after_ms().unwrap();
        assert!(first < 500, "first audio after {first} ms (loopback)");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(wait(Duration::from_secs(2), || r.status().contains("Studio")), "{}", r.status());
        let _ = r.shutdown().join();
        assert!(wait(Duration::from_secs(5), || phone.state.standby.load(Ordering::Acquire)));
    }
    assert_eq!(phone.state.woken.load(Ordering::Relaxed), 2);

    let mut c = config(phone.addr);
    c.wake = Some(WakeConfig { secret, counter: 150 });
    let r = Receiver::start(c, device(None)).unwrap();
    std::thread::sleep(Duration::from_millis(1200));
    assert!(!r.connected(), "WAKE with an old counter is rejected");
    let _ = r.shutdown().join();
    let _ = Pairing { secret, last_counter: 0, pc: None };
}
