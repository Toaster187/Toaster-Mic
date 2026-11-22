//! toaster-probe: test helper for Toaster Mic.
//!
//!   toaster-probe phone   [--bind IP] [--port N] [--channels 1-4] [--studio]
//!                         [--profile 0|1|2] [--secret HEX] [--standby]
//!                         [--always-on] [--seconds N]
//!   toaster-probe pc      --host IP [--port N] [--seconds N]  (receiver, no audio device)
//!   toaster-probe capture [--name "CABLE Output"] [--seconds N]   (Windows)
//!   toaster-probe devices                                       (Windows)

use std::net::Ipv4Addr;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use toaster_core::phone::Pairing;
use toaster_core::sim::{SimConfig, SimPhone};
use toaster_core::Profile;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}
fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("phone") => phone(&args),
        #[cfg(windows)]
        Some("pc") => pc(&args),
        #[cfg(windows)]
        Some("capture") => capture::run(&args),
        #[cfg(windows)]
        Some("devices") => capture::devices(),
        _ => {
            eprintln!("usage: toaster-probe phone|capture|devices [options]");
            std::process::exit(2);
        }
    }
}

fn unhex(s: &str) -> Option<[u8; 32]> {
    let v: Option<Vec<u8>> = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect();
    v?.try_into().ok()
}

fn phone(args: &[String]) {
    let pairing = arg(args, "--secret").and_then(|s| unhex(&s)).map(|secret| Pairing { secret, last_counter: 0, pc: None });
    let config = SimConfig {
        bind: arg(args, "--bind").and_then(|s| s.parse().ok()).unwrap_or(Ipv4Addr::UNSPECIFIED),
        port: arg(args, "--port").and_then(|s| s.parse().ok()).unwrap_or(toaster_core::wire::PHONE_PORT),
        channels: arg(args, "--channels").and_then(|s| s.parse().ok()).unwrap_or(1),
        studio: flag(args, "--studio"),
        profile: Profile::from_index(arg(args, "--profile").and_then(|s| s.parse().ok()).unwrap_or(0)),
        standby: flag(args, "--standby"),
        always_on: flag(args, "--always-on") || flag(args, "--standby"),
        pairing,
        ..Default::default()
    };
    let seconds: u64 = arg(args, "--seconds").and_then(|s| s.parse().ok()).unwrap_or(3600);
    let sim = SimPhone::start(config).expect("UDP-Port belegt?");
    println!("phone listening on {}", sim.addr);
    let end = Instant::now() + Duration::from_secs(seconds);
    let mut last = (0, 0, 0);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(500));
        let s = &sim.state;
        let now = (s.hellos.load(Ordering::Relaxed), s.woken.load(Ordering::Relaxed), s.stopped.load(Ordering::Relaxed));
        if now != last {
            last = now;
            println!(
                "hello={} woken={} stopped={} packets={} active={} standby={} wake_to_first_packet_ms={}",
                now.0,
                now.1,
                now.2,
                s.packets.load(Ordering::Relaxed),
                s.active.load(Ordering::Relaxed),
                s.standby.load(Ordering::Relaxed),
                s.first_packet_after_wake_ms.load(Ordering::Relaxed)
            );
        }
    }
}

#[cfg(windows)]
mod capture {
    use super::*;
    use toaster_windows::audio::{com_init, endpoint_name, endpoints, enumerator};
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::CLSCTX_ALL;

    pub fn devices() {
        eprintln!("devices start");
        for (label, flow) in [("Wiedergabe", eRender), ("Aufnahme", eCapture)] {
            let list = endpoints(flow);
            if let Err(e) = &list {
                eprintln!("{label}: Fehler {e:?}");
            }
            for e in list.unwrap_or_default() {
                println!("{label}: {} | {}", e.name, e.id);
            }
        }
    }

    /// Records like a meeting app would and reports how long the first audio
    /// took and the longest gap once audio flowed.
    pub fn run(args: &[String]) {
        let name = arg(args, "--name").unwrap_or_else(|| "CABLE Output".into()).to_ascii_lowercase();
        let seconds: u64 = arg(args, "--seconds").and_then(|s| s.parse().ok()).unwrap_or(20);
        let threshold: i32 = arg(args, "--threshold").and_then(|s| s.parse().ok()).unwrap_or(64);
        com_init();
        unsafe {
            let devices = enumerator().unwrap();
            let list = devices.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE).unwrap();
            let device = (0..list.GetCount().unwrap())
                .filter_map(|i| list.Item(i).ok())
                .find(|d| endpoint_name(d).to_ascii_lowercase().contains(&name))
                .expect("Aufnahmegerät nicht gefunden");
            println!("capturing from {}", endpoint_name(&device));
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None).unwrap();
            let format = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_PCM as u16,
                nChannels: 2,
                nSamplesPerSec: 48_000,
                nAvgBytesPerSec: 192_000,
                nBlockAlign: 4,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                    1_000_000,
                    0,
                    &format,
                    None,
                )
                .unwrap();
            let cap: IAudioCaptureClient = client.GetService().unwrap();
            let opened = Instant::now();
            client.Start().unwrap();
            let mut first: Option<f64> = None;
            let (mut frames_total, mut silent_run, mut max_gap, mut gaps) = (0u64, 0u64, 0u64, 0u32);
            let mut gap_log = Vec::new();
            while opened.elapsed() < Duration::from_secs(seconds) {
                std::thread::sleep(Duration::from_millis(5));
                loop {
                    let n = cap.GetNextPacketSize().unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    let mut data = std::ptr::null_mut();
                    let (mut frames, mut flags) = (0u32, 0u32);
                    if cap.GetBuffer(&mut data, &mut frames, &mut flags, None, None).is_err() {
                        break;
                    }
                    let silent_flag = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
                    let samples = std::slice::from_raw_parts(data as *const i16, frames as usize * 2);
                    for f in samples.chunks_exact(2) {
                        let loud = !silent_flag && (f[0] as i32).abs().max((f[1] as i32).abs()) > threshold;
                        frames_total += 1;
                        if loud {
                            if first.is_none() {
                                first = Some(opened.elapsed().as_secs_f64() * 1000.0);
                            }
                            if silent_run > 0 && first.is_some() {
                                if silent_run > 960 {
                                    gaps += 1;
                                    gap_log.push(format!("{:.1}s:{}ms", frames_total as f64 / 48000.0, silent_run / 48));
                                }
                                max_gap = max_gap.max(silent_run);
                            }
                            silent_run = 0;
                        } else if first.is_some() {
                            silent_run += 1;
                        }
                    }
                    let _ = cap.ReleaseBuffer(frames);
                }
            }
            let _ = client.Stop();
            println!(
                "first_audio_ms={} max_gap_ms={} gaps_over_20ms={} gap_positions=[{}]",
                first.map_or("-".into(), |v| format!("{v:.0}")),
                max_gap / 48,
                gaps,
                gap_log.join(", ")
            );
        }
    }
}

/// Runs the real Windows receiver against a phone (or emulator) without an
/// audio device and prints what arrives.
#[cfg(windows)]
fn pc(args: &[String]) {
    use std::sync::{Arc, Mutex};
    use toaster_windows::receiver::{Config, Receiver};
    let host: Ipv4Addr = arg(args, "--host").and_then(|s| s.parse().ok()).expect("--host IP");
    let port: u16 = arg(args, "--port").and_then(|s| s.parse().ok()).unwrap_or(toaster_core::wire::PHONE_PORT);
    let seconds: u64 = arg(args, "--seconds").and_then(|s| s.parse().ok()).unwrap_or(10);
    let output: toaster_windows::receiver::OutputFactory = Box::new(|playout: Arc<Mutex<toaster_core::Playout>>, _| {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let s = stop.clone();
        let t = std::thread::spawn(move || {
            let mut buf = vec![0i16; 960];
            while !s.load(Ordering::Acquire) {
                playout.lock().unwrap().read(&mut buf);
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        struct Sink(Arc<std::sync::atomic::AtomicBool>, Option<std::thread::JoinHandle<()>>);
        impl Drop for Sink {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
                if let Some(t) = self.1.take() { let _ = t.join(); }
            }
        }
        Ok(Box::new(Sink(stop, Some(t))) as Box<dyn Send>)
    });
    let secret = arg(args, "--secret").and_then(|s| unhex(&s));
    let counter: u64 = arg(args, "--counter").and_then(|s| s.parse().ok()).unwrap_or(1);
    let wake = secret.filter(|_| flag(args, "--wake")).map(|secret| toaster_windows::receiver::WakeConfig { secret, counter });
    let pair = secret.filter(|_| flag(args, "--pair")).map(|s| (s, counter));
    let config = Config {
        host, port, target_ms: 15, gain: 1.0, muted: false, wake, pair,
        studio: Some(Box::new(toaster_dsp_stub())),
    };
    let r = Receiver::start(config, output).expect("Empfänger");
    let end = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_secs(1));
        let s = r.snapshot();
        println!("{} | paired={} packets={} lost={} buffer={}ms target={}ms peak={} first_audio_ms={:?}",
            r.status(), r.paired(), s.stats.received, s.stats.lost, s.stats.buffered_ms, s.target_ms, s.peak, r.first_audio_after_ms());
    }
    let _ = r.shutdown().join();
}

#[cfg(windows)]
fn toaster_dsp_stub() -> toaster_core::Passthrough {
    toaster_core::Passthrough
}
