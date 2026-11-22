//! `libtoaster_engine.so`: loaded only by the `:audio` process (MicService).
//! JNI surface of `de.toaster.mic.Native`.

#[cfg(target_os = "android")]
mod engine;
#[cfg(target_os = "android")]
mod sender;

#[cfg(target_os = "android")]
mod jni_api {
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex, OnceLock};

    use jni::objects::{GlobalRef, JClass, JObject, JValue};
    use jni::sys::{jboolean, jint, JNI_TRUE};
    use jni::{JNIEnv, JavaVM};
    use toaster_android_audio::device;
    use toaster_android_audio::status::{Status, StatusCode, StatusWriter};
    use toaster_android_audio::PhoneSettings;

    use crate::engine::{self, Engine, Setup};

    static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);
    static VM: OnceLock<JavaVM> = OnceLock::new();
    static CLASS: OnceLock<GlobalRef> = OnceLock::new();

    fn notify(event: i32, profile: i32) {
        let (Some(vm), Some(class)) = (VM.get(), CLASS.get()) else { return };
        let Ok(mut env) = vm.attach_current_thread_as_daemon() else { return };
        let class = unsafe { JClass::from_raw(class.as_obj().as_raw()) };
        let _ = env.call_static_method(&class, "onEngineEvent", "(II)V", &[JValue::Int(event), JValue::Int(profile)]);
        let _ = env.exception_clear();
        std::mem::forget(class);
    }

    /// Returns 0 on success; otherwise the reason is in the status file.
    #[no_mangle]
    pub extern "system" fn Java_de_toaster_mic_Native_start(mut env: JNIEnv, class: JClass, context: JObject, standby: jboolean) -> jint {
        if let Ok(vm) = env.get_java_vm() {
            let _ = VM.set(vm);
        }
        if CLASS.get().is_none() {
            if let Ok(g) = env.new_global_ref(&class) {
                let _ = CLASS.set(g);
            }
        }
        let mut slot = ENGINE.lock().unwrap();
        if let Some(mut old) = slot.take() {
            old.stop();
        }
        let Ok(files) = device::files_dir(&mut env, &context) else { return 1 };
        let settings = PhoneSettings::load(&files);
        let build = device::build_info(&mut env);
        let devices = device::input_devices(&mut env, &context).unwrap_or_default();
        let _ = env.exception_clear();
        let mics = device::builtin_microphones(&mut env, &context);
        let unprocessed = device::unprocessed_supported(&mut env, &context);
        let (plan, fallback) = engine::plan(&settings, &build, &devices, mics, unprocessed);
        let setup = Setup { files: files.clone(), settings, plan, fallback };
        match Engine::start(setup, standby == JNI_TRUE, Arc::new(notify)) {
            Ok(e) => {
                *slot = Some(e);
                0
            }
            Err(message) => {
                let mut w = StatusWriter::open(&files);
                w.write(&Status { code: StatusCode::Error, message: format!("Fehler: {message}"), ..Default::default() });
                2
            }
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_de_toaster_mic_Native_stop(_env: JNIEnv, _class: JClass) {
        if let Some(mut e) = ENGINE.lock().unwrap().take() {
            e.stop();
        }
    }

    fn with_shared(f: impl FnOnce(&engine::Shared)) {
        if let Some(e) = ENGINE.lock().unwrap().as_ref() {
            f(&e.shared);
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_de_toaster_mic_Native_setMuted(_env: JNIEnv, _class: JClass, muted: jboolean) {
        with_shared(|s| s.muted.store(muted == JNI_TRUE, Ordering::Relaxed));
    }

    #[no_mangle]
    pub extern "system" fn Java_de_toaster_mic_Native_setGain(_env: JNIEnv, _class: JClass, percent: jint) {
        with_shared(|s| s.gain_pct.store(percent.clamp(0, 200) as u32, Ordering::Relaxed));
    }

    #[no_mangle]
    pub extern "system" fn Java_de_toaster_mic_Native_setLeft(_env: JNIEnv, _class: JClass, left: jint) {
        with_shared(|s| s.left.store(left.clamp(0, 1) as u8, Ordering::Relaxed));
    }

    #[no_mangle]
    pub extern "system" fn Java_de_toaster_mic_Native_setLive(_env: JNIEnv, _class: JClass, live: jboolean) {
        with_shared(|s| s.live.store(live == JNI_TRUE, Ordering::Relaxed));
    }

    #[no_mangle]
    pub extern "system" fn Java_de_toaster_mic_Native_networkChanged(_env: JNIEnv, _class: JClass) {
        with_shared(|s| {
            s.announce.store(true, Ordering::Release);
            s.wake();
        });
    }

    #[no_mangle]
    pub extern "system" fn Java_de_toaster_mic_Native_isRunning(_env: JNIEnv, _class: JClass) -> jboolean {
        ENGINE.lock().unwrap().is_some() as jboolean
    }
}
