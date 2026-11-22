//! Offline checks of the studio chain with a simulated 3-microphone phone.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use nnnoiseless::DenoiseState;
use toaster_core::sim::MicArray;
use toaster_core::wire::flags;
use toaster_core::Processor;
use toaster_dsp::studio::{CEILING, NS_DELAY};
use toaster_dsp::{NoiseLevel, Studio};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
thread_local! {
    // Only allocations of the measuring thread count (tests run in parallel).
    static COUNT_HERE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if COUNT_HERE.with(|c| c.get()) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
}
#[global_allocator]
static A: Counting = Counting;

const FRAMES: usize = 160; // 3-channel packets

fn run(studio: &mut Studio, mics: &mut MicArray, packets: usize, flag: u8, gain: f32) -> Vec<f32> {
    let mut input = vec![0i16; FRAMES * 3];
    let mut out = vec![0f32; FRAMES * 2];
    let mut mono = Vec::with_capacity(packets * FRAMES);
    for _ in 0..packets {
        mics.fill(&mut input);
        studio.process(&input, 3, flags::STUDIO | flag, gain, &mut out);
        mono.extend(out.chunks_exact(2).map(|f| f[0]));
    }
    mono
}

fn power(x: &[f32]) -> f64 {
    x.iter().map(|&v| v as f64 * v as f64).sum::<f64>() / x.len() as f64
}

#[test]
fn estimates_microphone_delays() {
    let mut studio = Studio::new(NoiseLevel::Medium);
    let mut mics = MicArray::new(3, 220.0, 3000.0, 150.0);
    mics.delays = [0, 7, 15, 0];
    mics.gains = [1.0, 0.8, 0.45, 0.0];
    run(&mut studio, &mut mics, 900, 0, 1.0); // 3 s
    let d = studio.delays();
    assert_eq!(studio.reference(), 0, "the loudest, cleanest microphone is the reference");
    assert!((d[1] - 7.0).abs() <= 1.0 && (d[2] - 15.0).abs() <= 1.0, "delays {d:?}");
}

/// Linear path only: frozen weights, voice and noise measured separately.
fn snr_gain(gains: [f32; 4]) -> (f64, f64) {
    let mut studio = Studio::new(NoiseLevel::Off);
    studio.linear = true;
    let mut mics = MicArray::new(3, 220.0, 3000.0, 600.0);
    mics.delays = [0, 6, 11, 0];
    mics.gains = gains;
    run(&mut studio, &mut mics, 900, 0, 1.0);
    studio.frozen = true;
    mics.noise_on = false;
    let voice = run(&mut studio, &mut mics, 300, 0, 1.0);
    mics.noise_on = true;
    mics.voice_on = false;
    let noise = run(&mut studio, &mut mics, 300, 0, 1.0);
    let skip = 2 * 480;
    let out = power(&voice[skip..]) / power(&noise[skip..]);
    // Best single microphone: gain 1 voice over the same white noise.
    let single = (gains[0] as f64 * 3000.0).powi(2) * 0.5 / (600.0f64.powi(2) / 3.0);
    let _ = single;
    let theory: f64 = gains[..3].iter().map(|g| (*g as f64).powi(2)).sum::<f64>() / (gains[0] as f64).powi(2);
    (10.0 * (out / (out / theory)).log10(), 10.0 * out.log10())
}

#[test]
fn combining_three_microphones_improves_snr() {
    // Measure the SNR of the best microphone alone through the same linear path.
    let single_db = {
        let mut studio = Studio::new(NoiseLevel::Off);
        studio.linear = true;
        let mut mics = MicArray::new(3, 220.0, 3000.0, 600.0);
        mics.gains = [1.0, 0.0, 0.0, 0.0];
        mics.delays = [0, 0, 0, 0];
        // Two dead channels: only microphone 1 contributes.
        run(&mut studio, &mut mics, 900, 0, 1.0);
        studio.frozen = true;
        mics.noise_on = false;
        let v = run(&mut studio, &mut mics, 300, 0, 1.0);
        mics.noise_on = true;
        mics.voice_on = false;
        let n = run(&mut studio, &mut mics, 300, 0, 1.0);
        10.0 * (power(&v[960..]) / power(&n[960..])).log10()
    };
    let (_, equal_db) = snr_gain([1.0, 0.95, 0.9, 0.0]);
    let (_, weak_db) = snr_gain([1.0, 0.8, 0.45, 0.0]);
    let theory_equal = 10.0 * (1.0f64 + 0.9025 + 0.81).log10();
    let theory_weak = 10.0 * (1.0f64 + 0.64 + 0.2025).log10();
    println!("single {single_db:.2} dB, three similar mics {equal_db:.2} dB (+{:.2}, theory +{theory_equal:.2}), with weak mic {weak_db:.2} dB (+{:.2}, theory +{theory_weak:.2})", equal_db - single_db, weak_db - single_db);
    assert!(equal_db - single_db >= 4.0, "similar microphones: at least +4 dB");
    assert!(weak_db - single_db >= theory_weak - 0.7, "weak microphone does not drag the result down");
}

#[test]
fn output_never_exceeds_minus_one_dbfs() {
    let mut studio = Studio::new(NoiseLevel::Strong);
    let mut mics = MicArray::new(3, 180.0, 30000.0, 2000.0);
    let out = run(&mut studio, &mut mics, 900, 0, 10.0);
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak <= CEILING + 0.5, "peak {peak}");
    assert!(peak > CEILING * 0.5, "loud input stays loud");
}

#[test]
fn quiet_raw_microphones_become_loud() {
    // Raw UNPROCESSED capture on the Pixel is around -60 dBFS.
    let mut studio = Studio::new(NoiseLevel::Medium);
    let mut mics = MicArray::new(3, 200.0, 120.0, 6.0);
    let out = run(&mut studio, &mut mics, 1500, 0, 1.0);
    let tail = &out[out.len() - 48_000..];
    let rms = power(tail).sqrt();
    let dbfs = 20.0 * (rms / 32768.0).log10();
    println!("output level {dbfs:.1} dBFS, AGC {:.1} dB", 20.0 * studio.agc_gain().log10());
    assert!(dbfs > -30.0, "level {dbfs:.1} dBFS");
}

#[test]
fn fast_start_crossfade_is_seamless() {
    // A switches from fast start to the full chain; B is always full, C always fast.
    let (mut a, mut b, mut c) = (Studio::new(NoiseLevel::Medium), Studio::new(NoiseLevel::Medium), Studio::new(NoiseLevel::Medium));
    let mic = || MicArray::new(3, 220.0, 3000.0, 150.0);
    let (mut ma, mut mb, mut mc) = (mic(), mic(), mic());
    let mut ra = run(&mut a, &mut ma, 600, flags::FAST_START, 1.0);
    assert_eq!(a.fade(), 0.0, "fast path while the flag is set");
    ra.extend(run(&mut a, &mut ma, 600, 0, 1.0));
    assert_eq!(a.fade(), 1.0);
    let rb = run(&mut b, &mut mb, 1200, 0, 1.0);
    let rc = run(&mut c, &mut mc, 1200, flags::FAST_START, 1.0);
    let switch = 600 * FRAMES;
    let mut worst = f64::MAX;
    for s in (switch - 4800..switch + 4800).step_by(120) {
        let rms = |x: &[f32]| power(&x[s..s + 120]).sqrt();
        let expected = rms(&rb).min(rms(&rc));
        if expected > 50.0 {
            worst = worst.min(rms(&ra) / expected);
        }
    }
    println!("lowest level ratio around the switch: {worst:.2}");
    assert!(worst > 0.5, "no dip when leaving the fast start");
    // After the crossfade (and compressor release) the output is the full chain.
    let tail = switch + 24_000;
    let diff: Vec<f32> = ra[tail..tail + 4800].iter().zip(&rb[tail..tail + 4800]).map(|(x, y)| x - y).collect();
    let rel = power(&diff).sqrt() / power(&rb[tail..tail + 4800]).sqrt();
    assert!(rel < 0.02, "difference to the full chain {rel:.3}");
}

#[test]
fn rnnoise_delay_matches_constant() {
    let mut ns = DenoiseState::new();
    let mut voice = toaster_core::sim::Voice::new(210.0);
    let input: Vec<f32> = (0..48_000).map(|_| voice.next() * 8000.0).collect();
    let mut output = vec![0f32; input.len()];
    for (i, o) in input.chunks_exact(480).zip(output.chunks_exact_mut(480)) {
        ns.process_frame(o, i);
    }
    let best = (0..1000)
        .max_by(|&a, &b| {
            let c = |lag: usize| input[..40_000].iter().zip(&output[lag..lag + 40_000]).map(|(x, y)| x * y).sum::<f32>();
            c(a).total_cmp(&c(b))
        })
        .unwrap();
    assert!((best as isize - NS_DELAY as isize).abs() <= 2, "RNNoise delay {best}");
}

#[test]
fn processing_does_not_allocate_and_is_fast() {
    let mut studio = Studio::new(NoiseLevel::Strong);
    let mut mics = MicArray::new(3, 220.0, 3000.0, 150.0);
    let mut input = vec![0i16; FRAMES * 3];
    let mut out = vec![0f32; FRAMES * 2];
    for _ in 0..300 {
        mics.fill(&mut input);
        studio.process(&input, 3, flags::STUDIO, 1.0, &mut out);
    }
    let before = ALLOCS.load(Ordering::Relaxed);
    COUNT_HERE.with(|c| c.set(true));
    let start = std::time::Instant::now();
    let packets = 3000; // 10 s
    for _ in 0..packets {
        mics.fill(&mut input);
        studio.process(&input, 3, flags::STUDIO, 1.0, &mut out);
    }
    COUNT_HERE.with(|c| c.set(false));
    let per_block = start.elapsed().as_secs_f64() * 1000.0 / (packets as f64 / 3.0);
    let allocs = ALLOCS.load(Ordering::Relaxed) - before;
    println!("{per_block:.3} ms per 10-ms block, {allocs} allocations");
    assert_eq!(allocs, 0, "real-time path allocates");
    let budget = if cfg!(debug_assertions) { 5.0 } else { 1.0 };
    assert!(per_block < budget, "{per_block:.3} ms per block");
}
