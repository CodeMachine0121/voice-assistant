//! The only code that talks to real audio devices. Thin on purpose: device choice,
//! stream construction and error translation; all sound maths lives elsewhere.
use crate::capture::Capture;
use crate::recorder::StreamSource;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    Device, ErrorKind, FromSample, Sample as _, SampleFormat, SizedSample, Stream, StreamConfig,
};
use std::sync::{Arc, Mutex};
use voice_core::Problem;

/// Opens the real microphone.
pub struct HardwareSource;

/// Keeps the cpal stream alive; dropping it stops capture.
pub struct LiveStream {
    _stream: Stream,
}

impl StreamSource for HardwareSource {
    type Handle = LiveStream;

    fn open(
        &self,
        device: Option<&str>,
        capture: Arc<Mutex<Capture>>,
    ) -> Result<LiveStream, Problem> {
        let device = choose_device(device)?;
        let supported = device.default_input_config().map_err(problem_from)?;
        let config = supported.config();
        capture
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .set_rate(config.sample_rate);
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config, capture),
            SampleFormat::F64 => build::<f64>(&device, config, capture),
            SampleFormat::I16 => build::<i16>(&device, config, capture),
            SampleFormat::I32 => build::<i32>(&device, config, capture),
            SampleFormat::I8 => build::<i8>(&device, config, capture),
            SampleFormat::U16 => build::<u16>(&device, config, capture),
            SampleFormat::U8 => build::<u8>(&device, config, capture),
            other => {
                return Err(Problem::Other(format!(
                    "unsupported microphone format: {other}"
                )))
            }
        }?;
        stream.play().map_err(problem_from)?;
        Ok(LiveStream { _stream: stream })
    }
}

/// Names of all input devices; empty when the audio system is unavailable.
pub fn input_device_names() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|devices| devices.filter_map(|d| device_name(&d)).collect())
        .unwrap_or_default()
}

fn device_name(device: &Device) -> Option<String> {
    device.description().ok().map(|d| d.name().to_string())
}

fn choose_device(wanted: Option<&str>) -> Result<Device, Problem> {
    let host = cpal::default_host();
    if let Some(name) = wanted {
        let found = host
            .input_devices()
            .ok()
            .and_then(|mut all| all.find(|d| device_name(d).as_deref() == Some(name)));
        match found {
            Some(device) => return Ok(device),
            None => log::warn!("microphone '{name}' not found, using the system default"),
        }
    }
    host.default_input_device()
        .ok_or_else(|| Problem::Other("No microphone found".into()))
}

fn build<T>(
    device: &Device,
    config: StreamConfig,
    capture: Arc<Mutex<Capture>>,
) -> Result<Stream, Problem>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], _| {
                let mono_ready: Vec<f32> = data.iter().map(|s| f32::from_sample(*s)).collect();
                capture
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push_interleaved(&mono_ready, channels);
            },
            |err| log::error!("microphone stream error: {err}"),
            None,
        )
        .map_err(problem_from)
}

fn problem_from(err: cpal::Error) -> Problem {
    problem_for(err.kind(), &err.to_string())
}

/// Translates a backend failure into what the user is told.
pub(crate) fn problem_for(kind: ErrorKind, message: &str) -> Problem {
    match kind {
        ErrorKind::PermissionDenied => Problem::MicrophonePermission,
        _ => Problem::Other(message.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denied_access_means_microphone_permission() {
        assert_eq!(
            problem_for(ErrorKind::PermissionDenied, "x"),
            Problem::MicrophonePermission
        );
    }

    #[test]
    fn any_other_failure_keeps_its_message() {
        assert_eq!(
            problem_for(ErrorKind::DeviceBusy, "busy"),
            Problem::Other("busy".into())
        );
    }

    /// Needs a microphone and permission; run with `cargo test -p voice-audio -- --ignored`.
    #[test]
    #[ignore]
    fn real_microphone_records_something() {
        use crate::CpalRecorder;
        use voice_core::Recorder;
        let mut recorder = CpalRecorder::new(None);
        recorder.start().expect("microphone available");
        std::thread::sleep(std::time::Duration::from_millis(600));
        let recording = recorder.stop();
        assert!(recording.samples.len() > 4_000);
    }
}
