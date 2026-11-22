//! WASAPI: endpoint enumeration and an event-driven shared-mode render stream
//! (10 ms, PCM16 stereo 48 kHz, Windows converts to the mix format).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use toaster_core::Playout;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW, WaitForSingleObject,
};

use crate::receiver::{FaultSink, OutputFactory};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub name: String,
    pub id: String,
}

/// Initialise COM (MTA) for the current thread; safe to call repeatedly.
pub fn com_init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

pub fn enumerator() -> windows::core::Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

pub fn endpoint_name(device: &IMMDevice) -> String {
    unsafe {
        device
            .OpenPropertyStore(STGM_READ)
            .and_then(|store| store.GetValue(&PKEY_Device_FriendlyName))
            .map(|v| v.to_string())
            .unwrap_or_default()
    }
}

pub fn endpoint_id(device: &IMMDevice) -> String {
    unsafe {
        match device.GetId() {
            Ok(p) => {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as *const _));
                s
            }
            Err(_) => String::new(),
        }
    }
}

pub fn endpoints(flow: EDataFlow) -> windows::core::Result<Vec<Endpoint>> {
    com_init();
    let e = enumerator()?;
    let mut out = Vec::new();
    unsafe {
        let collection = e.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        for i in 0..collection.GetCount()? {
            let device = collection.Item(i)?;
            out.push(Endpoint { name: endpoint_name(&device), id: endpoint_id(&device) });
        }
    }
    Ok(out)
}

pub fn render_endpoints() -> windows::core::Result<Vec<Endpoint>> {
    endpoints(eRender)
}

/// The VB-CABLE playback side, preselected as audio target.
pub fn is_cable_input(name: &str) -> bool {
    name.to_ascii_lowercase().contains("cable input")
}

struct OwnedEvent(HANDLE);
unsafe impl Send for OwnedEvent {}
impl Drop for OwnedEvent {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub struct Output {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Output {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub fn output_factory(device_id: String) -> OutputFactory {
    Box::new(move |playout, fault| {
        let out = start(&device_id, playout, fault)?;
        Ok(Box::new(out) as Box<dyn Send>)
    })
}

pub fn start(device_id: &str, playout: Arc<Mutex<Playout>>, fault: FaultSink) -> std::io::Result<Output> {
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = mpsc::channel();
    let id = device_id.to_string();
    let thread = {
        let stop = stop.clone();
        std::thread::Builder::new().name("toaster-audio".into()).spawn(move || {
            if let Err(e) = render(&id, &playout, &stop, &ready_tx) {
                if ready_tx.send(Err(e.message().to_string())).is_err() && !stop.load(Ordering::Acquire) {
                    fault(e.message().to_string());
                }
            }
        })?
    };
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(Output { stop, thread: Some(thread) }),
        Ok(Err(m)) => {
            let _ = thread.join();
            Err(std::io::Error::other(m))
        }
        Err(_) => Err(std::io::Error::other("Audioausgabe konnte nicht gestartet werden.")),
    }
}

fn render(
    device_id: &str,
    playout: &Mutex<Playout>,
    stop: &AtomicBool,
    ready: &mpsc::Sender<Result<(), String>>,
) -> windows::core::Result<()> {
    com_init();
    unsafe {
        let device = enumerator()?.GetDevice(&HSTRING::from(device_id))?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let format = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM as u16,
            nChannels: 2,
            nSamplesPerSec: 48_000,
            nAvgBytesPerSec: 48_000 * 4,
            nBlockAlign: 4,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            100_000, // 10 ms in 100-ns units
            0,
            &format,
            None,
        )?;
        let event = OwnedEvent(CreateEventW(None, false, false, PCWSTR::null())?);
        client.SetEventHandle(event.0)?;
        let capacity = client.GetBufferSize()?;
        let render: IAudioRenderClient = client.GetService()?;
        render.GetBuffer(capacity)?;
        render.ReleaseBuffer(capacity, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
        let mut task = 0u32;
        let mmcss = AvSetMmThreadCharacteristicsW(windows::core::w!("Pro Audio"), &mut task).ok();
        client.Start()?;
        let _ = ready.send(Ok(()));
        let mut scratch = vec![0i16; capacity as usize * 2];
        let result = (|| -> windows::core::Result<()> {
            while !stop.load(Ordering::Acquire) {
                if WaitForSingleObject(event.0, 200) != WAIT_OBJECT_0 {
                    continue;
                }
                let padding = client.GetCurrentPadding()?;
                let frames = capacity.saturating_sub(padding);
                if frames == 0 {
                    continue;
                }
                let out = &mut scratch[..frames as usize * 2];
                playout.lock().unwrap().read(out);
                let dst = render.GetBuffer(frames)?;
                std::ptr::copy_nonoverlapping(out.as_ptr() as *const u8, dst, out.len() * 2);
                render.ReleaseBuffer(frames, 0)?;
            }
            Ok(())
        })();
        let _ = client.Stop();
        if let Some(h) = mmcss {
            let _ = AvRevertMmThreadCharacteristics(h);
        }
        result
    }
}

