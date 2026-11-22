//! Android framework queries through JNI (only at start-up, never per packet).

use std::path::PathBuf;

use jni::objects::{JObject, JObjectArray, JString, JValue};
use jni::JNIEnv;

pub const TYPE_BUILTIN_MIC: i32 = 15;
const MIC_TYPES: [i32; 7] = [15, 3, 11, 12, 22, 7, 26];

#[derive(Clone, Debug)]
pub struct InputDevice {
    pub id: i32,
    pub kind: i32,
    pub address: String,
    pub name: String,
}

impl InputDevice {
    pub fn label(&self) -> String {
        let kind = match self.kind {
            15 => "internes Mikrofon",
            11 | 12 | 22 => "USB",
            3 => "Headset",
            7 | 26 => "Bluetooth",
            _ => "Audioeingang",
        };
        format!("{} · {} {}", self.name, kind, self.address).trim().to_string()
    }
}

#[derive(Clone, Debug, Default)]
pub struct BuildInfo {
    pub manufacturer: String,
    pub device: String,
    pub fingerprint: String,
    pub sdk: i32,
}

impl BuildInfo {
    /// Google Pixel 9 ("tokay"): three raw microphones behind the "bottom" device.
    pub fn is_pixel9(&self) -> bool {
        self.manufacturer.eq_ignore_ascii_case("google") && self.device == "tokay"
    }
}

fn jstring(env: &mut JNIEnv, obj: JObject) -> String {
    if obj.is_null() {
        return String::new();
    }
    let s = JString::from(obj);
    env.get_string(&s).map(|s| s.into()).unwrap_or_default()
}

fn static_string(env: &mut JNIEnv, class: &str, field: &str) -> String {
    match env.get_static_field(class, field, "Ljava/lang/String;").and_then(|v| v.l()) {
        Ok(o) => jstring(env, o),
        Err(_) => {
            let _ = env.exception_clear();
            String::new()
        }
    }
}

pub fn build_info(env: &mut JNIEnv) -> BuildInfo {
    let sdk = env
        .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")
        .and_then(|v| v.i())
        .unwrap_or(28);
    let _ = env.exception_clear();
    BuildInfo {
        manufacturer: static_string(env, "android/os/Build", "MANUFACTURER"),
        device: static_string(env, "android/os/Build", "DEVICE"),
        fingerprint: static_string(env, "android/os/Build", "FINGERPRINT"),
        sdk,
    }
}

pub fn files_dir(env: &mut JNIEnv, context: &JObject) -> jni::errors::Result<PathBuf> {
    let file = env.call_method(context, "getFilesDir", "()Ljava/io/File;", &[])?.l()?;
    let path = env.call_method(&file, "getAbsolutePath", "()Ljava/lang/String;", &[])?.l()?;
    Ok(PathBuf::from(jstring(env, path)))
}

fn audio_manager<'a>(env: &mut JNIEnv<'a>, context: &JObject) -> jni::errors::Result<JObject<'a>> {
    let name = env.new_string("audio")?;
    env.call_method(context, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&name)])?.l()
}

/// Microphone-type input devices (built-in, USB, headset, Bluetooth).
pub fn input_devices(env: &mut JNIEnv, context: &JObject) -> jni::errors::Result<Vec<InputDevice>> {
    let am = audio_manager(env, context)?;
    let array: JObjectArray = env
        .call_method(&am, "getDevices", "(I)[Landroid/media/AudioDeviceInfo;", &[JValue::Int(1)])?
        .l()?
        .into();
    let n = env.get_array_length(&array)?;
    let mut out = Vec::new();
    for i in 0..n {
        let d = env.get_object_array_element(&array, i)?;
        let kind = env.call_method(&d, "getType", "()I", &[])?.i()?;
        if !MIC_TYPES.contains(&kind) {
            continue;
        }
        let id = env.call_method(&d, "getId", "()I", &[])?.i()?;
        let address = env.call_method(&d, "getAddress", "()Ljava/lang/String;", &[])?.l()?;
        let address = jstring(env, address);
        let name = env.call_method(&d, "getProductName", "()Ljava/lang/CharSequence;", &[])?.l()?;
        let name = if name.is_null() {
            String::new()
        } else {
            let s = env.call_method(&name, "toString", "()Ljava/lang/String;", &[])?.l()?;
            jstring(env, s)
        };
        out.push(InputDevice { id, kind, address, name });
    }
    Ok(out)
}

/// Number of built-in microphones the HAL reports (Android 9+).
pub fn builtin_microphones(env: &mut JNIEnv, context: &JObject) -> usize {
    let r = (|| -> jni::errors::Result<usize> {
        let am = audio_manager(env, context)?;
        let list = env.call_method(&am, "getMicrophones", "()Ljava/util/List;", &[])?.l()?;
        let n = env.call_method(&list, "size", "()I", &[])?.i()?;
        let mut count = 0;
        for i in 0..n {
            let m = env.call_method(&list, "get", "(I)Ljava/lang/Object;", &[JValue::Int(i)])?.l()?;
            if env.call_method(&m, "getType", "()I", &[])?.i()? == TYPE_BUILTIN_MIC {
                count += 1;
            }
        }
        Ok(count)
    })();
    let _ = env.exception_clear();
    r.unwrap_or(0)
}

pub fn unprocessed_supported(env: &mut JNIEnv, context: &JObject) -> bool {
    let r = (|| -> jni::errors::Result<bool> {
        let am = audio_manager(env, context)?;
        let key = env.new_string("android.media.property.SUPPORT_AUDIO_SOURCE_UNPROCESSED")?;
        let v = env.call_method(&am, "getProperty", "(Ljava/lang/String;)Ljava/lang/String;", &[JValue::Object(&key)])?.l()?;
        Ok(jstring(env, v) == "true")
    })();
    let _ = env.exception_clear();
    r.unwrap_or(false)
}

/// The built-in microphone device with the given address ("bottom" on Pixel).
pub fn builtin(devices: &[InputDevice], address: Option<&str>) -> Option<InputDevice> {
    devices
        .iter()
        .filter(|d| d.kind == TYPE_BUILTIN_MIC)
        .find(|d| address.is_none_or(|a| d.address == a))
        .cloned()
}

/// Local IPv4 addresses of up, non-loopback interfaces (for display).
pub fn local_ipv4() -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) != 0 {
            return out;
        }
        let mut cur = ifap;
        while !cur.is_null() {
            let ifa = &*cur;
            if !ifa.ifa_addr.is_null()
                && (*ifa.ifa_addr).sa_family as i32 == libc::AF_INET
                && ifa.ifa_flags & libc::IFF_UP as u32 != 0
                && ifa.ifa_flags & libc::IFF_LOOPBACK as u32 == 0
            {
                let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                if ip.is_private() && !out.contains(&ip.to_string()) {
                    out.push(ip.to_string());
                }
            }
            cur = ifa.ifa_next;
        }
        libc::freeifaddrs(ifap);
    }
    out
}
