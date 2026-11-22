//! JNI access for the UI process: activity, `de.toaster.mic.Bridge` helpers.

use std::path::PathBuf;

use jni::objects::{GlobalRef, JClass, JObject, JValue};
use jni::{AttachGuard, JavaVM};
use slint::android::AndroidApp;
use toaster_android_audio::device::{self, BuildInfo, InputDevice};

pub const START: &str = "de.toaster.mic.START";
pub const STANDBY: &str = "de.toaster.mic.STANDBY";
pub const STOP: &str = "de.toaster.mic.STOP";
pub const SET: &str = "de.toaster.mic.SET";

pub struct Java {
    vm: JavaVM,
    activity: GlobalRef,
    bridge: GlobalRef,
}

impl Java {
    pub fn new(app: &AndroidApp) -> jni::errors::Result<Java> {
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _)? };
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        let (activity, bridge) = {
            let mut env = vm.attach_current_thread()?;
            let global = env.new_global_ref(&activity)?;
            // Native threads see only the system class loader; use the app's.
            let loader = env.call_method(&activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
            let name = env.new_string("de.toaster.mic.Bridge")?;
            let class = env
                .call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name)])?
                .l()?;
            (global, env.new_global_ref(class)?)
        };
        Ok(Java { vm, activity, bridge })
    }

    fn env(&self) -> AttachGuard<'_> {
        self.vm.attach_current_thread().expect("JVM attach")
    }

    fn call_bool(&self, method: &str) -> bool {
        let mut env = self.env();
        let class: &JClass = self.bridge.as_obj().into();
        let r = env
            .call_static_method(class, method, "(Landroid/content/Context;)Z", &[JValue::Object(self.activity.as_obj())])
            .and_then(|v| v.z())
            .unwrap_or(false);
        let _ = env.exception_clear();
        r
    }

    fn call_activity(&self, method: &str) {
        let mut env = self.env();
        let class: &JClass = self.bridge.as_obj().into();
        let _ = env.call_static_method(class, method, "(Landroid/app/Activity;)V", &[JValue::Object(self.activity.as_obj())]);
        let _ = env.exception_clear();
    }

    /// Sends an intent to MicService in the `:audio` process.
    pub fn send(&self, action: &str, key: &str, value: i32) {
        let mut env = self.env();
        let class: &JClass = self.bridge.as_obj().into();
        if let (Ok(a), Ok(k)) = (env.new_string(action), env.new_string(key)) {
            let _ = env.call_static_method(
                class,
                "send",
                "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;I)V",
                &[JValue::Object(self.activity.as_obj()), JValue::Object(&a), JValue::Object(&k), JValue::Int(value)],
            );
        }
        let _ = env.exception_clear();
    }

    pub fn can_record(&self) -> bool {
        self.call_bool("canRecord")
    }
    pub fn dnd_access(&self) -> bool {
        self.call_bool("dndAccess")
    }
    pub fn request_permissions(&self) {
        self.call_activity("requestPermissions");
    }
    pub fn open_dnd(&self) {
        self.call_activity("openDndAccess");
    }
    pub fn open_battery(&self) {
        self.call_activity("openBatterySettings");
    }

    pub fn files_dir(&self) -> PathBuf {
        let mut env = self.env();
        let r = device::files_dir(&mut env, self.activity.as_obj()).unwrap_or_else(|_| PathBuf::from("/data/data/de.toaster.mic/files"));
        let _ = env.exception_clear();
        r
    }
    pub fn devices(&self) -> Vec<InputDevice> {
        let mut env = self.env();
        let r = device::input_devices(&mut env, self.activity.as_obj()).unwrap_or_default();
        let _ = env.exception_clear();
        r
    }
    pub fn build(&self) -> BuildInfo {
        device::build_info(&mut self.env())
    }
    pub fn builtin_microphones(&self) -> usize {
        device::builtin_microphones(&mut self.env(), self.activity.as_obj())
    }
}
