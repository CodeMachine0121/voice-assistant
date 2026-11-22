//! Background engine: owns the controller and recorder, runs the pipeline,
//! and publishes `Status` updates. The UI only sends shortcut events and settings.
use crate::{
    Command, ControllerEvent, DictationPipeline, Recorder, SessionController, Settings,
    ShortcutEvent, Status,
};
use crossbeam_channel::{never, select, tick, unbounded, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

/// How often the engine ticks the controller (maximum length) and refreshes the input level.
pub const TICK_INTERVAL: Duration = Duration::from_millis(100);

pub struct EngineDeps {
    pub recorder: Box<dyn Recorder>,
    pub pipeline: DictationPipeline,
    pub settings: Settings,
}

/// Handle used by the UI and the shortcut listener.
#[derive(Clone)]
pub struct EngineHandle {
    pub shortcuts: Sender<ShortcutEvent>,
    pub settings: Sender<Settings>,
    pub status: Receiver<Status>,
}

pub struct Engine;

impl Engine {
    /// Spawns the engine thread. `on_status` is called after every status change
    /// (e.g. to request a UI repaint).
    ///
    /// The thread stops once every sender of shortcut events has been dropped.
    ///
    /// # Panics
    /// When the operating system refuses to create a thread (as `std::thread::spawn` does);
    /// the app cannot work without its engine.
    pub fn spawn(deps: EngineDeps, on_status: Box<dyn Fn() + Send>) -> EngineHandle {
        let (shortcuts_tx, shortcuts) = unbounded();
        let (settings_tx, settings) = unbounded();
        let (status_tx, status_rx) = unbounded();
        let worker = Worker {
            controller: SessionController::new(),
            recorder: deps.recorder,
            pipeline: deps.pipeline,
            settings: deps.settings,
            shortcuts,
            status: status_tx,
            on_status,
        };
        thread::Builder::new()
            .name("voice-engine".into())
            .spawn(move || worker.run(settings))
            .unwrap_or_else(|e| panic!("cannot start the dictation engine thread: {e}"));
        EngineHandle {
            shortcuts: shortcuts_tx,
            settings: settings_tx,
            status: status_rx,
        }
    }
}

struct Worker {
    controller: SessionController,
    recorder: Box<dyn Recorder>,
    pipeline: DictationPipeline,
    settings: Settings,
    shortcuts: Receiver<ShortcutEvent>,
    status: Sender<Status>,
    on_status: Box<dyn Fn() + Send>,
}

impl Worker {
    fn run(mut self, mut settings: Receiver<Settings>) {
        let ticker = tick(TICK_INTERVAL);
        loop {
            select! {
                recv(self.shortcuts) -> event => match event {
                    Ok(event) => self.handle(ControllerEvent::Shortcut(event)),
                    Err(_) => break,
                },
                recv(settings) -> update => match update {
                    Ok(update) => self.settings = update,
                    // Nobody can send settings any more: keep the current ones.
                    Err(_) => settings = never(),
                },
                recv(ticker) -> _ => self.handle(ControllerEvent::Tick),
            }
        }
        if self.controller.is_listening() {
            self.recorder.cancel();
        }
    }

    fn handle(&mut self, event: ControllerEvent) {
        match self.controller.handle(event, Instant::now()) {
            Some(command) => self.execute(command),
            None if self.controller.is_listening() && event == ControllerEvent::Tick => {
                self.publish(Status::Listening {
                    level: self.recorder.level(),
                });
            }
            None => {}
        }
    }

    fn execute(&mut self, command: Command) {
        match command {
            Command::StartRecording => match self.recorder.start() {
                Ok(()) => {
                    self.publish(Status::Listening {
                        level: self.recorder.level(),
                    });
                }
                Err(problem) => {
                    // The dictation never began: forget it entirely.
                    self.controller
                        .handle(ControllerEvent::RecordingFailed, Instant::now());
                    self.publish(Status::Problem(problem));
                }
            },
            Command::StopAndProcess => {
                self.publish(Status::Working);
                let recording = self.recorder.stop();
                let outcome = self.pipeline.process(recording, &self.settings);
                // Only one dictation at a time: shortcuts pressed meanwhile are dropped.
                while self.shortcuts.try_recv().is_ok() {}
                self.publish(outcome.status);
                self.controller
                    .handle(ControllerEvent::ProcessingFinished, Instant::now());
            }
            Command::Discard => {
                self.recorder.cancel();
                self.publish(Status::Idle);
            }
            Command::Cancel => {
                self.recorder.cancel();
                self.publish(Status::Cancelled);
            }
        }
    }

    fn publish(&self, status: Status) {
        // The UI may already be gone while the engine winds down; nothing to tell then.
        let _ = self.status.send(status);
        (self.on_status)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{speech, FakeInserter, FakePolisher, FakeStore, FakeTranscriber};
    use crate::{History, PolishError, Problem, Recording};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use ShortcutEvent::{EscapePressed, HandsFreePressed, PushToTalkDown, PushToTalkUp};

    const WAIT: Duration = Duration::from_secs(5);

    #[derive(Default)]
    struct RecorderLog {
        starts: usize,
        stops: usize,
        cancels: usize,
    }

    struct FakeRecorder {
        log: Arc<Mutex<RecorderLog>>,
        start_result: Result<(), Problem>,
        level: f32,
    }

    impl Recorder for FakeRecorder {
        fn start(&mut self) -> Result<(), Problem> {
            self.log.lock().unwrap().starts += 1;
            self.start_result.clone()
        }
        fn stop(&mut self) -> Recording {
            self.log.lock().unwrap().stops += 1;
            speech()
        }
        fn cancel(&mut self) {
            self.log.lock().unwrap().cancels += 1;
        }
        fn level(&self) -> f32 {
            self.level
        }
    }

    /// Transcriber that takes a while, so events can arrive during processing.
    struct SlowTranscriber(Duration);

    impl crate::Transcriber for SlowTranscriber {
        fn transcribe(
            &self,
            _: &Recording,
            _: &crate::PersonalDictionary,
        ) -> Result<String, String> {
            thread::sleep(self.0);
            Ok("hello team".into())
        }
    }

    struct Rig {
        handle: EngineHandle,
        recorder: Arc<Mutex<RecorderLog>>,
        inserter: Arc<FakeInserter>,
        store: Arc<FakeStore>,
        notified: Arc<AtomicUsize>,
    }

    struct Options {
        start_result: Result<(), Problem>,
        transcriber: Arc<dyn crate::Transcriber>,
    }

    impl Default for Options {
        fn default() -> Self {
            Self {
                start_result: Ok(()),
                transcriber: FakeTranscriber::new(Ok("hello team")),
            }
        }
    }

    impl Rig {
        fn new(options: Options) -> Self {
            let recorder = Arc::new(Mutex::new(RecorderLog::default()));
            let inserter = FakeInserter::new(Ok(()));
            let store = FakeStore::new(History::default());
            let notified = Arc::new(AtomicUsize::new(0));
            let pipeline = DictationPipeline::new(
                options.transcriber,
                FakePolisher::new(Ok("Hello team.")),
                inserter.clone(),
                store.clone(),
            );
            let counter = notified.clone();
            let handle = Engine::spawn(
                EngineDeps {
                    recorder: Box::new(FakeRecorder {
                        log: recorder.clone(),
                        start_result: options.start_result,
                        level: 0.42,
                    }),
                    pipeline,
                    settings: Settings::default(),
                },
                Box::new(move || {
                    counter.fetch_add(1, Ordering::SeqCst);
                }),
            );
            Self {
                handle,
                recorder,
                inserter,
                store,
                notified,
            }
        }

        fn press(&self, event: ShortcutEvent) {
            self.handle.shortcuts.send(event).unwrap();
        }

        /// Next status that is not a live level update.
        /// Gives up after `WAIT` in total, so endless level updates cannot hang a test.
        fn next_status(&self) -> Status {
            let deadline = Instant::now() + WAIT;
            loop {
                match self.handle.status.recv_deadline(deadline).expect("status") {
                    Status::Listening { .. } => continue,
                    other => return other,
                }
            }
        }

        fn expect_listening(&self) {
            assert_eq!(
                self.handle.status.recv_timeout(WAIT).expect("status"),
                Status::Listening { level: 0.42 }
            );
        }

        fn log<T>(&self, f: impl FnOnce(&RecorderLog) -> T) -> T {
            f(&self.recorder.lock().unwrap())
        }

        /// Drops every sender and collects the remaining statuses until the thread exits.
        fn shut_down(self) -> (Vec<Status>, usize, Arc<Mutex<RecorderLog>>) {
            let status = self.handle.status.clone();
            drop(self.handle);
            let mut rest = Vec::new();
            loop {
                match status.recv_timeout(WAIT) {
                    Ok(s) => rest.push(s),
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                    Err(e) => panic!("engine did not stop: {e}"),
                }
            }
            (rest, self.notified.load(Ordering::SeqCst), self.recorder)
        }
    }

    fn hold(duration: Duration) {
        thread::sleep(duration);
    }

    #[test]
    fn push_to_talk_records_processes_and_reports_done() {
        let rig = Rig::new(Options::default());

        rig.press(PushToTalkDown);
        rig.expect_listening();
        hold(Duration::from_millis(550));
        rig.press(PushToTalkUp);

        assert_eq!(rig.next_status(), Status::Working);
        assert_eq!(rig.next_status(), Status::Done);
        assert_eq!(rig.inserter.inserted(), ["Hello team."]);
        assert_eq!(rig.log(|l| (l.starts, l.stops, l.cancels)), (1, 1, 0));
    }

    #[test]
    fn listening_publishes_the_live_level_on_each_tick() {
        let rig = Rig::new(Options::default());

        rig.press(HandsFreePressed);
        rig.expect_listening();

        // the next update arrives from the clock, not from a shortcut
        rig.expect_listening();
        rig.expect_listening();
    }

    #[test]
    fn a_short_press_is_discarded_back_to_idle() {
        let rig = Rig::new(Options::default());

        rig.press(PushToTalkDown);
        rig.expect_listening();
        rig.press(PushToTalkUp);

        assert_eq!(rig.next_status(), Status::Idle);
        assert_eq!(rig.log(|l| (l.stops, l.cancels)), (0, 1));
        assert!(rig.inserter.inserted().is_empty());
    }

    #[test]
    fn escape_cancels_and_records_nothing() {
        let rig = Rig::new(Options::default());

        rig.press(HandsFreePressed);
        rig.expect_listening();
        rig.press(EscapePressed);

        assert_eq!(rig.next_status(), Status::Cancelled);
        assert_eq!(rig.log(|l| (l.stops, l.cancels)), (0, 1));
        assert!(rig.inserter.inserted().is_empty());
        assert!(rig.store.saved().is_empty());
    }

    #[test]
    fn missing_microphone_permission_is_reported_and_a_new_attempt_is_allowed() {
        let rig = Rig::new(Options {
            start_result: Err(Problem::MicrophonePermission),
            ..Options::default()
        });

        rig.press(PushToTalkDown);
        assert_eq!(
            rig.next_status(),
            Status::Problem(Problem::MicrophonePermission)
        );
        rig.press(HandsFreePressed);
        assert_eq!(
            rig.next_status(),
            Status::Problem(Problem::MicrophonePermission)
        );

        assert_eq!(rig.log(|l| (l.starts, l.stops)), (2, 0));
    }

    #[test]
    fn a_failed_start_publishes_no_live_level_and_cancels_nothing_on_shutdown() {
        let rig = Rig::new(Options {
            start_result: Err(Problem::MicrophonePermission),
            ..Options::default()
        });

        rig.press(PushToTalkDown);
        assert_eq!(
            rig.handle.status.recv_timeout(WAIT).expect("status"),
            Status::Problem(Problem::MicrophonePermission)
        );
        hold(TICK_INTERVAL * 3);

        let (rest, _, recorder) = rig.shut_down();
        assert!(rest.is_empty(), "unexpected statuses: {rest:?}");
        assert_eq!(recorder.lock().unwrap().cancels, 0);
    }

    #[test]
    fn shortcuts_pressed_while_working_are_ignored() {
        let rig = Rig::new(Options {
            transcriber: Arc::new(SlowTranscriber(Duration::from_millis(300))),
            ..Options::default()
        });

        rig.press(HandsFreePressed);
        rig.expect_listening();
        hold(Duration::from_millis(550));
        rig.press(HandsFreePressed);
        assert_eq!(rig.next_status(), Status::Working);
        rig.press(PushToTalkDown);
        rig.press(HandsFreePressed);
        assert_eq!(rig.next_status(), Status::Done);
        hold(Duration::from_millis(250));

        let (rest, _, recorder) = rig.shut_down();
        assert_eq!(recorder.lock().unwrap().starts, 1);
        assert!(rest.is_empty(), "unexpected statuses: {rest:?}");
    }

    #[test]
    fn new_settings_apply_to_the_next_dictation() {
        let rig = Rig::new(Options::default());
        rig.handle
            .settings
            .send(Settings {
                cleanup_enabled: false,
                ..Settings::default()
            })
            .unwrap();
        hold(Duration::from_millis(50));

        rig.press(PushToTalkDown);
        hold(Duration::from_millis(550));
        rig.press(PushToTalkUp);
        assert_eq!(rig.next_status(), Status::Working);
        assert_eq!(rig.next_status(), Status::Done);

        assert_eq!(rig.inserter.inserted(), ["hello team"]);
    }

    #[test]
    fn engine_keeps_working_when_only_the_settings_sender_is_gone() {
        let rig = Rig::new(Options::default());
        let EngineHandle {
            shortcuts,
            settings,
            status,
        } = rig.handle.clone();
        drop(settings);
        let Rig { handle, .. } = rig;
        drop(handle);
        hold(Duration::from_millis(50));

        shortcuts.send(PushToTalkDown).unwrap();
        assert_eq!(
            status.recv_timeout(WAIT).unwrap(),
            Status::Listening { level: 0.42 }
        );
    }

    #[test]
    fn on_status_is_called_after_every_status() {
        let rig = Rig::new(Options::default());
        rig.press(PushToTalkDown);
        rig.press(EscapePressed);
        let mut seen = 0;
        loop {
            seen += 1;
            if rig.handle.status.recv_timeout(WAIT).unwrap() == Status::Cancelled {
                break;
            }
        }

        let (rest, notified, _) = rig.shut_down();

        assert_eq!(notified, seen + rest.len());
    }

    #[test]
    fn dropping_the_handle_stops_the_engine_and_releases_the_microphone() {
        let rig = Rig::new(Options::default());
        rig.press(HandsFreePressed);
        rig.expect_listening();

        let (_, _, recorder) = rig.shut_down();

        assert_eq!(recorder.lock().unwrap().cancels, 1);
    }

    #[test]
    fn unreachable_assistant_is_published_as_the_final_status() {
        let recorder = Arc::new(Mutex::new(RecorderLog::default()));
        let handle = Engine::spawn(
            EngineDeps {
                recorder: Box::new(FakeRecorder {
                    log: recorder,
                    start_result: Ok(()),
                    level: 0.0,
                }),
                pipeline: DictationPipeline::new(
                    FakeTranscriber::new(Ok("hello there")),
                    FakePolisher::new(Err(PolishError::Unreachable("down".into()))),
                    FakeInserter::new(Ok(())),
                    FakeStore::new(History::default()),
                ),
                settings: Settings::default(),
            },
            Box::new(|| {}),
        );

        handle.shortcuts.send(PushToTalkDown).unwrap();
        thread::sleep(Duration::from_millis(550));
        handle.shortcuts.send(PushToTalkUp).unwrap();

        let last = std::iter::from_fn(|| handle.status.recv_timeout(WAIT).ok())
            .find(|s| matches!(s, Status::Problem(_) | Status::Done));
        assert_eq!(last, Some(Status::Problem(Problem::AssistantUnreachable)));
    }
}
