//! `Recorder` implementation: lifecycle of one capture, independent of the audio backend.
use crate::capture::Capture;
use crate::hardware::HardwareSource;
use crossbeam_channel::{bounded, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use voice_core::{Problem, Recorder, Recording};

/// Opens a live stream feeding a [`Capture`]. The returned handle keeps the stream alive and
/// stops it when dropped. It never has to be `Send`: it is created, held and dropped on the
/// dedicated capture thread (cpal streams cannot cross threads on macOS).
pub trait StreamSource: Send + Sync + 'static {
    type Handle;
    /// `device` is the preferred microphone; `None` or an unknown name means the system default.
    fn open(
        &self,
        device: Option<&str>,
        capture: Arc<Mutex<Capture>>,
    ) -> Result<Self::Handle, Problem>;
}

struct Session {
    stop: Sender<()>,
    thread: JoinHandle<()>,
    capture: Arc<Mutex<Capture>>,
}

impl Session {
    /// Ends the stream and waits for the capture thread to release the microphone.
    fn finish(self) -> Arc<Mutex<Capture>> {
        let _ = self.stop.send(());
        let _ = self.thread.join();
        self.capture
    }
}

fn locked(capture: &Mutex<Capture>) -> MutexGuard<'_, Capture> {
    capture
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Records from a microphone through `cpal`.
pub struct CpalRecorder<S: StreamSource = HardwareSource> {
    source: Arc<S>,
    device: Option<String>,
    session: Option<Session>,
}

impl CpalRecorder<HardwareSource> {
    /// `device_name` of `None` records from the system default microphone.
    pub fn new(device_name: Option<String>) -> Self {
        Self::with_source(HardwareSource, device_name)
    }

    /// Names of the microphones that can be chosen, for the settings screen.
    pub fn list_input_devices() -> Vec<String> {
        crate::hardware::input_device_names()
    }
}

impl<S: StreamSource> CpalRecorder<S> {
    pub fn with_source(source: S, device_name: Option<String>) -> Self {
        Self {
            source: Arc::new(source),
            device: device_name,
            session: None,
        }
    }

    /// Chooses the microphone for the next recording.
    pub fn set_device(&mut self, device_name: Option<String>) {
        self.device = device_name;
    }
}

impl<S: StreamSource> Recorder for CpalRecorder<S> {
    fn start(&mut self) -> Result<(), Problem> {
        if self.session.is_some() {
            return Ok(());
        }
        let capture = Arc::new(Mutex::new(Capture::default()));
        let (ready_tx, ready_rx) = bounded::<Result<(), Problem>>(1);
        let (stop_tx, stop_rx) = bounded::<()>(1);
        let thread = {
            let source = Arc::clone(&self.source);
            let device = self.device.clone();
            let capture = Arc::clone(&capture);
            std::thread::spawn(move || match source.open(device.as_deref(), capture) {
                Ok(handle) => {
                    let _ = ready_tx.send(Ok(()));
                    let _ = stop_rx.recv();
                    drop(handle);
                }
                Err(problem) => {
                    let _ = ready_tx.send(Err(problem));
                }
            })
        };
        let session = Session {
            stop: stop_tx,
            thread,
            capture,
        };
        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.session = Some(session);
                Ok(())
            }
            Ok(Err(problem)) => {
                session.finish();
                Err(problem)
            }
            Err(_) => {
                session.finish();
                Err(Problem::Other("audio thread stopped unexpectedly".into()))
            }
        }
    }

    fn stop(&mut self) -> Recording {
        match self.session.take() {
            Some(session) => {
                let capture = session.finish();
                let recording = locked(&capture).take_recording();
                recording
            }
            None => Recording::default(),
        }
    }

    fn cancel(&mut self) {
        if let Some(session) = self.session.take() {
            session.finish();
        }
    }

    fn level(&self) -> f32 {
        self.session
            .as_ref()
            .map_or(0.0, |s| locked(&s.capture).level())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// Delivers a fixed burst of 16 kHz mono sound when opened.
    struct FakeSource {
        samples: Vec<f32>,
        fail_with: Option<Problem>,
        panic: bool,
        opened_with: Mutex<Vec<Option<String>>>,
        live_handles: Arc<AtomicUsize>,
        released: Arc<AtomicBool>,
    }

    impl FakeSource {
        fn speaking(samples: Vec<f32>) -> Self {
            Self {
                samples,
                fail_with: None,
                panic: false,
                opened_with: Mutex::default(),
                live_handles: Arc::default(),
                released: Arc::default(),
            }
        }
        fn failing(problem: Problem) -> Self {
            Self {
                fail_with: Some(problem),
                ..Self::speaking(vec![])
            }
        }
    }

    struct FakeHandle(Arc<AtomicUsize>, Arc<AtomicBool>);
    impl Drop for FakeHandle {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
            self.1.store(true, Ordering::SeqCst);
        }
    }

    impl StreamSource for Arc<FakeSource> {
        type Handle = FakeHandle;
        fn open(
            &self,
            device: Option<&str>,
            capture: Arc<Mutex<Capture>>,
        ) -> Result<FakeHandle, Problem> {
            if self.panic {
                panic!("backend crashed");
            }
            self.opened_with
                .lock()
                .unwrap()
                .push(device.map(String::from));
            if let Some(problem) = &self.fail_with {
                return Err(problem.clone());
            }
            let mut c = capture.lock().unwrap();
            c.set_rate(16_000);
            c.push_interleaved(&self.samples, 1);
            self.live_handles.fetch_add(1, Ordering::SeqCst);
            Ok(FakeHandle(self.live_handles.clone(), self.released.clone()))
        }
    }

    fn recorder(source: &Arc<FakeSource>, device: Option<&str>) -> CpalRecorder<Arc<FakeSource>> {
        CpalRecorder::with_source(source.clone(), device.map(String::from))
    }

    #[test]
    fn stop_returns_what_was_heard() {
        let source = Arc::new(FakeSource::speaking(vec![0.25; 1_600]));
        let mut rec = recorder(&source, None);
        rec.start().unwrap();
        assert_eq!(rec.stop().samples, vec![0.25; 1_600]);
    }

    #[test]
    fn stop_releases_the_microphone() {
        let source = Arc::new(FakeSource::speaking(vec![0.1; 10]));
        let mut rec = recorder(&source, None);
        rec.start().unwrap();
        rec.stop();
        assert_eq!(source.live_handles.load(Ordering::SeqCst), 0);
        assert!(source.released.load(Ordering::SeqCst));
    }

    #[test]
    fn cancel_releases_the_microphone_and_keeps_nothing() {
        let source = Arc::new(FakeSource::speaking(vec![0.1; 10]));
        let mut rec = recorder(&source, None);
        rec.start().unwrap();
        rec.cancel();
        assert_eq!(source.live_handles.load(Ordering::SeqCst), 0);
        assert!(rec.stop().samples.is_empty());
    }

    #[test]
    fn cancel_without_recording_does_nothing() {
        let source = Arc::new(FakeSource::speaking(vec![]));
        recorder(&source, None).cancel();
        assert!(source.opened_with.lock().unwrap().is_empty());
    }

    #[test]
    fn stop_without_start_returns_an_empty_recording() {
        let source = Arc::new(FakeSource::speaking(vec![0.5; 10]));
        assert_eq!(recorder(&source, None).stop(), Recording::default());
    }

    #[test]
    fn denied_microphone_reports_permission_problem_and_records_nothing() {
        let source = Arc::new(FakeSource::failing(Problem::MicrophonePermission));
        let mut rec = recorder(&source, None);
        assert_eq!(rec.start(), Err(Problem::MicrophonePermission));
        assert!(rec.stop().samples.is_empty());
        assert_eq!(rec.level(), 0.0);
    }

    #[test]
    fn other_failures_are_passed_through() {
        let source = Arc::new(FakeSource::failing(Problem::Other("no microphone".into())));
        assert_eq!(
            recorder(&source, None).start(),
            Err(Problem::Other("no microphone".into()))
        );
    }

    #[test]
    fn a_crashing_backend_becomes_a_problem() {
        let mut fake = FakeSource::speaking(vec![]);
        fake.panic = true;
        let source = Arc::new(fake);
        assert!(matches!(
            recorder(&source, None).start(),
            Err(Problem::Other(_))
        ));
    }

    #[test]
    fn starting_twice_keeps_a_single_stream() {
        let source = Arc::new(FakeSource::speaking(vec![0.1; 10]));
        let mut rec = recorder(&source, None);
        rec.start().unwrap();
        rec.start().unwrap();
        assert_eq!(source.opened_with.lock().unwrap().len(), 1);
    }

    #[test]
    fn level_is_zero_when_not_recording() {
        let source = Arc::new(FakeSource::speaking(vec![1.0; 800]));
        assert_eq!(recorder(&source, None).level(), 0.0);
    }

    #[test]
    fn level_follows_live_sound_while_recording() {
        let source = Arc::new(FakeSource::speaking(vec![1.0; 800]));
        let mut rec = recorder(&source, None);
        rec.start().unwrap();
        assert_eq!(rec.level(), 1.0);
        rec.stop();
        assert_eq!(rec.level(), 0.0);
    }

    #[test]
    fn chosen_device_is_requested_on_start() {
        let source = Arc::new(FakeSource::speaking(vec![]));
        let mut rec = recorder(&source, Some("USB Mic"));
        rec.start().unwrap();
        rec.stop();
        rec.set_device(None);
        rec.start().unwrap();
        rec.stop();
        assert_eq!(
            *source.opened_with.lock().unwrap(),
            vec![Some("USB Mic".to_string()), None]
        );
    }

    #[test]
    fn recorder_can_move_to_another_thread() {
        fn assert_send<T: Send>() {}
        assert_send::<CpalRecorder<Arc<FakeSource>>>();
        assert_send::<CpalRecorder>();
    }

    #[test]
    fn recording_can_be_made_again_after_stopping() {
        let source = Arc::new(FakeSource::speaking(vec![0.5; 160]));
        let mut rec = recorder(&source, None);
        rec.start().unwrap();
        rec.stop();
        rec.start().unwrap();
        assert_eq!(rec.stop().samples.len(), 160);
    }

    #[test]
    fn a_new_hardware_recorder_is_idle() {
        let mut rec = CpalRecorder::new(Some("Desk Mic".into()));
        assert_eq!(rec.level(), 0.0);
        assert_eq!(rec.stop(), Recording::default());
    }

    #[test]
    fn listing_devices_never_fails() {
        // Contents depend on the machine; an absent audio system must give an empty list.
        let _ = CpalRecorder::list_input_devices();
    }
}
