//! Small Windows integrations: single instance, autostart registry entry.

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, INFINITE};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("ToasterMic");

/// `true` if this is the first instance. Otherwise the running instance is
/// asked to show its window.
pub fn claim_single_instance() -> bool {
    unsafe {
        let _mutex = CreateMutexW(None, true, w!("Local\\ToasterMic.Instance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, w!("Local\\ToasterMic.Show")) {
                let _ = SetEvent(event);
            }
            return false;
        }
        // The mutex handle is intentionally kept for the process lifetime.
        std::mem::forget(_mutex);
        true
    }
}

/// Calls `on_show` whenever a second instance is started. The thread sleeps
/// on a kernel event and costs nothing while waiting.
pub fn on_second_instance(on_show: impl Fn() + Send + 'static) {
    unsafe {
        let Ok(event) = CreateEventW(None, false, false, w!("Local\\ToasterMic.Show")) else { return };
        let event = SendHandle(event);
        std::thread::Builder::new()
            .name("toaster-instance".into())
            .spawn(move || {
                let event = event;
                loop {
                    WaitForSingleObject(event.0, INFINITE);
                    on_show();
                }
            })
            .ok();
    }
}

struct SendHandle(HANDLE);
unsafe impl Send for SendHandle {}

pub fn set_autostart(enabled: bool) -> windows::core::Result<()> {
    unsafe {
        if enabled {
            let exe = std::env::current_exe().map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()))?;
            let command = format!("\"{}\" --tray", exe.display());
            let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
            RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, REG_SZ.0, Some(wide.as_ptr() as *const _), (wide.len() * 2) as u32)
                .ok()
        } else {
            let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
            if r.is_ok() || r == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                r.ok()
            }
        }
    }
}

pub fn hstring(s: &str) -> HSTRING {
    HSTRING::from(s)
}
