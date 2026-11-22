//! Always-On: event-driven detection of programs that record from the
//! virtual microphone ("CABLE Output"), plus the ANNOUNCE listener.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use toaster_core::wire::{self, ANNOUNCE};
use toaster_core::ControlKey;
use windows::core::{implement, Interface, Ref, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, PROPERTYKEY};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};

use crate::audio::{com_init, endpoint_name, enumerator};

/// The VB-CABLE recording side that programs select as microphone.
pub fn is_cable_output(name: &str) -> bool {
    name.to_ascii_lowercase().contains("cable output")
}

#[derive(Clone, Copy)]
struct Signal(HANDLE);
unsafe impl Send for Signal {}
unsafe impl Sync for Signal {}
impl Signal {
    fn set(&self) {
        unsafe {
            let _ = SetEvent(self.0);
        }
    }
}

#[implement(IAudioSessionNotification)]
struct SessionCreated(Signal);
impl IAudioSessionNotification_Impl for SessionCreated_Impl {
    fn OnSessionCreated(&self, _s: Ref<IAudioSessionControl>) -> windows::core::Result<()> {
        self.0.set();
        Ok(())
    }
}

#[implement(IAudioSessionEvents)]
struct SessionEvents(Signal);
impl IAudioSessionEvents_Impl for SessionEvents_Impl {
    fn OnDisplayNameChanged(&self, _: &PCWSTR, _: *const windows::core::GUID) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnIconPathChanged(&self, _: &PCWSTR, _: *const windows::core::GUID) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnSimpleVolumeChanged(&self, _: f32, _: windows::core::BOOL, _: *const windows::core::GUID) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnChannelVolumeChanged(&self, _: u32, _: *const f32, _: u32, _: *const windows::core::GUID) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnGroupingParamChanged(&self, _: *const windows::core::GUID, _: *const windows::core::GUID) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnStateChanged(&self, _: AudioSessionState) -> windows::core::Result<()> {
        self.0.set();
        Ok(())
    }
    fn OnSessionDisconnected(&self, _: AudioSessionDisconnectReason) -> windows::core::Result<()> {
        self.0.set();
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceEvents(Signal);
impl IMMNotificationClient_Impl for DeviceEvents_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
        self.0.set();
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        self.0.set();
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
        self.0.set();
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, _: EDataFlow, _: ERole, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}

pub struct MicMonitor {
    stop: Arc<AtomicBool>,
    signal: Signal,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for MicMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.signal.set();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        unsafe {
            let _ = CloseHandle(self.signal.0);
        }
    }
}

/// Calls `on_change(in_use)` whenever the number of active recording
/// sessions on CABLE Output switches between zero and non-zero. Sleeps on a
/// Win32 event between notifications; a 30-s resync guards against lost events.
pub fn watch(on_change: impl Fn(bool) + Send + 'static) -> windows::core::Result<MicMonitor> {
    let event = unsafe { CreateEventW(None, false, false, PCWSTR::null())? };
    let signal = Signal(event);
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("toaster-micwatch".into())
            .spawn(move || {
                com_init();
                let mut last: Option<bool> = None;
                let Ok(devices) = enumerator() else { return };
                let device_events: IMMNotificationClient = DeviceEvents(signal).into();
                unsafe {
                    let _ = devices.RegisterEndpointNotificationCallback(&device_events);
                }
                let mut watched: Option<Watched> = None;
                while !stop.load(Ordering::Acquire) {
                    if watched.as_ref().is_none_or(|w| !w.alive()) {
                        watched = Watched::open(&devices, signal);
                    }
                    let in_use = watched.as_mut().is_some_and(|w| w.refresh());
                    if last != Some(in_use) {
                        last = Some(in_use);
                        on_change(in_use);
                    }
                    unsafe {
                        WaitForSingleObject(signal.0, 30_000);
                    }
                }
                drop(watched);
                unsafe {
                    let _ = devices.UnregisterEndpointNotificationCallback(&device_events);
                }
            })
            .map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()))?
    };
    Ok(MicMonitor { stop, signal, thread: Some(thread) })
}

struct Watched {
    device: IMMDevice,
    manager: IAudioSessionManager2,
    created: IAudioSessionNotification,
    signal: Signal,
    sessions: HashMap<String, (IAudioSessionControl, IAudioSessionEvents)>,
}

impl Watched {
    fn open(devices: &IMMDeviceEnumerator, signal: Signal) -> Option<Watched> {
        unsafe {
            let collection = devices.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE).ok()?;
            for i in 0..collection.GetCount().ok()? {
                let device = collection.Item(i).ok()?;
                if !is_cable_output(&endpoint_name(&device)) {
                    continue;
                }
                let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).ok()?;
                let created: IAudioSessionNotification = SessionCreated(signal).into();
                manager.RegisterSessionNotification(&created).ok()?;
                return Some(Watched { device, manager, created, signal, sessions: HashMap::new() });
            }
            None
        }
    }

    fn alive(&self) -> bool {
        unsafe { self.device.GetState().is_ok_and(|s| s == DEVICE_STATE_ACTIVE) }
    }

    /// Re-registers state callbacks for all sessions; true if any is recording.
    fn refresh(&mut self) -> bool {
        let mut active = false;
        let mut seen = HashMap::new();
        unsafe {
            // The enumerator must be requested for creation notifications to flow.
            let Ok(list) = self.manager.GetSessionEnumerator() else { return false };
            let count = list.GetCount().unwrap_or(0);
            for i in 0..count {
                let Ok(control) = list.GetSession(i) else { continue };
                let identity = session_identity(&control);
                let state = control.GetState().unwrap_or(AudioSessionStateExpired);
                if state == AudioSessionStateActive {
                    active = true;
                }
                let entry = match self.sessions.remove(&identity) {
                    Some(e) => e,
                    None => {
                        let events: IAudioSessionEvents = SessionEvents(self.signal).into();
                        let _ = control.RegisterAudioSessionNotification(&events);
                        (control, events)
                    }
                };
                seen.insert(identity, entry);
            }
        }
        for (_, (control, events)) in self.sessions.drain() {
            unsafe {
                let _ = control.UnregisterAudioSessionNotification(&events);
            }
        }
        self.sessions = seen;
        active
    }
}

fn session_identity(control: &IAudioSessionControl) -> String {
    unsafe {
        let Ok(c2) = control.cast::<IAudioSessionControl2>() else { return format!("{:p}", control.as_raw()) };
        match c2.GetSessionInstanceIdentifier() {
            Ok(p) => {
                let s = p.to_string().unwrap_or_default();
                windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
                s
            }
            Err(_) => format!("{:p}", control.as_raw()),
        }
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        unsafe {
            for (_, (control, events)) in self.sessions.drain() {
                let _ = control.UnregisterAudioSessionNotification(&events);
            }
            let _ = self.manager.UnregisterSessionNotification(&self.created);
        }
    }
}

/// Listens on UDP 8126 for authenticated ANNOUNCE datagrams of the paired
/// phone. Blocks without timeout; wakes only for received packets. The
/// pairing can be replaced or removed at any time.
pub struct AnnounceListener {
    pairing: Arc<Mutex<Option<(ControlKey, u64)>>>,
}

impl AnnounceListener {
    pub fn start(on_announce: impl Fn(Ipv4Addr, u16, u64) + Send + 'static) -> std::io::Result<AnnounceListener> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, wire::ANNOUNCE_PORT))?;
        let pairing: Arc<Mutex<Option<(ControlKey, u64)>>> = Arc::new(Mutex::new(None));
        let shared = pairing.clone();
        std::thread::Builder::new().name("toaster-announce".into()).spawn(move || {
            let mut buf = [0u8; 256];
            loop {
                let Ok((n, from)) = socket.recv_from(&mut buf) else { continue };
                let SocketAddr::V4(from) = from else { continue };
                let mut guard = shared.lock().unwrap();
                let Some((key, last)) = guard.as_mut() else { continue };
                let Some((h, payload)) = key.open(&buf[..n]) else { continue };
                if h.kind != ANNOUNCE || payload.len() != 2 || h.counter <= *last {
                    continue;
                }
                *last = h.counter;
                let port = u16::from_le_bytes([payload[0], payload[1]]);
                drop(guard);
                on_announce(*from.ip(), port, h.counter);
            }
        })?;
        Ok(AnnounceListener { pairing })
    }

    pub fn set_pairing(&self, pairing: Option<([u8; 32], u64)>) {
        *self.pairing.lock().unwrap() = pairing.map(|(s, c)| (ControlKey::new(&s), c));
    }
}
