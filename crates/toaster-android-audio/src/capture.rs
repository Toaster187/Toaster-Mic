//! AAudio input stream driven by a data callback (no blocking read thread).

use ndk::audio::{
    AudioCallbackResult, AudioDirection, AudioError, AudioFormat, AudioInputPreset, AudioPerformanceMode, AudioSharingMode,
    AudioStream, AudioStreamBuilder,
};

#[derive(Clone, Debug)]
pub struct CaptureConfig {
    /// 0 lets Android choose.
    pub device_id: i32,
    pub channels: i32,
    pub unprocessed: bool,
    pub low_latency: bool,
    /// Larger callbacks wake the CPU less often (battery profile).
    pub frames_per_callback: Option<i32>,
}

pub struct Capture {
    stream: AudioStream,
}

impl Capture {
    pub fn channel_count(&self) -> i32 {
        self.stream.channel_count()
    }
    pub fn device_id(&self) -> i32 {
        self.stream.device_id()
    }
    pub fn input_preset(&self) -> AudioInputPreset {
        self.stream.input_preset()
    }
}

// AAudio stream handles may be stopped and closed from any thread; the engine
// only moves ownership between its threads.
unsafe impl Send for Capture {}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.stream.request_stop();
    }
}

/// Opens and starts capturing. `on_audio` receives interleaved PCM16 frames on
/// AAudio's real-time thread and must not block or allocate.
pub fn start(
    config: &CaptureConfig,
    mut on_audio: impl FnMut(&[i16]) + Send + 'static,
    mut on_error: impl FnMut(String) + Send + 'static,
) -> Result<Capture, String> {
    let channels = config.channels;
    let mut builder = AudioStreamBuilder::new()
        .map_err(|e| format!("AAudio: {e:?}"))?
        .direction(AudioDirection::Input)
        .sample_rate(toaster_core::wire::RATE as i32)
        .channel_count(channels)
        .format(AudioFormat::PCM_I16)
        .sharing_mode(AudioSharingMode::Shared)
        .input_preset(if config.unprocessed { AudioInputPreset::Unprocessed } else { AudioInputPreset::VoiceRecognition })
        .performance_mode(if config.low_latency { AudioPerformanceMode::LowLatency } else { AudioPerformanceMode::None })
        .data_callback(Box::new(move |_stream, data, frames| {
            if frames > 0 && !data.is_null() {
                let samples = unsafe { std::slice::from_raw_parts(data as *const i16, frames as usize * channels as usize) };
                on_audio(samples);
            }
            AudioCallbackResult::Continue
        }))
        .error_callback(Box::new(move |_stream, error: AudioError| on_error(format!("{error:?}"))));
    if config.device_id > 0 {
        builder = builder.device_id(config.device_id);
    }
    if let Some(f) = config.frames_per_callback {
        builder = builder.frames_per_data_callback(f);
    }
    let stream = builder.open_stream().map_err(|e| format!("Aufnahme nicht verfügbar ({e:?})"))?;
    if stream.channel_count() != channels {
        return Err(format!("Android liefert {} statt {} Kanäle.", stream.channel_count(), channels));
    }
    stream.request_start().map_err(|e| format!("Android hat die Aufnahme nicht gestartet ({e:?})"))?;
    Ok(Capture { stream })
}
