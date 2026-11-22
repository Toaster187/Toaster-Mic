//! Pieces shared by the Android engine (`:audio` process) and the UI process:
//! settings file, status file, device/JNI and AAudio helpers.

pub mod settings;
pub mod status;

#[cfg(target_os = "android")]
pub mod capture;
#[cfg(target_os = "android")]
pub mod device;

pub use settings::{Mode, PairingFile, PhoneSettings};
pub use status::{Status, StatusCode};
