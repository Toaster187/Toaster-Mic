//! Toaster Mic for Windows: receives the phone microphone over WLAN and plays
//! it into VB-CABLE so that any program can use it as microphone.

pub mod receiver;
pub mod settings;

#[cfg(windows)]
pub mod audio;
#[cfg(windows)]
pub mod monitor;
#[cfg(windows)]
pub mod system;

/// 32x32 RGBA microphone icon drawn in code (no image files to ship).
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let mut px = vec![0u8; (size * size * 4) as usize];
    let s = size as f32;
    let accent = [0xBD, 0xF4, 0x77];
    let mut put = |x: u32, y: u32, a: f32| {
        let i = ((y * size + x) * 4) as usize;
        let a = (a.clamp(0.0, 1.0) * 255.0) as u8;
        if a > px[i + 3] {
            px[i..i + 3].copy_from_slice(&accent);
            px[i + 3] = a;
        }
    };
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
            // Capsule: rounded rectangle centred at x = 0.5.
            let (cx, half_w, top, bottom) = (0.5, 0.17, 0.12, 0.58);
            let dy = if fy < top + half_w { top + half_w - fy } else if fy > bottom - half_w { fy - (bottom - half_w) } else { 0.0 };
            let d = ((fx - cx).powi(2) + dy.powi(2)).sqrt();
            put(x, y, (half_w - d) * s + 0.5);
            // Holder arc.
            let r = ((fx - 0.5).powi(2) + (fy - 0.46).powi(2)).sqrt();
            if fy > 0.46 {
                put(x, y, (0.035 - (r - 0.27).abs()) * s + 0.5);
            }
            // Stem and base.
            if (fx - 0.5).abs() < 0.035 && fy > 0.73 && fy < 0.88 {
                put(x, y, 1.0);
            }
            if (fx - 0.5).abs() < 0.2 && (fy - 0.88).abs() < 0.035 {
                put(x, y, 1.0);
            }
        }
    }
    px
}
