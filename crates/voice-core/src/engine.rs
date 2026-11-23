//! Background engine: owns the controller and recorder, runs the pipeline,
//! and publishes `Status` updates. The UI only sends shortcut events and settings.
use crate::{
    Action, Command, ControllerEvent, DictationPipeline, PreparedAction, Problem, Recorder,
    SelectionReader, SessionController, Settings, ShortcutEvent, Status,
};
use crossbeam_channel::{never, select, tick, unbounded, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// How often the engine ticks the controller (maximum length) and refreshes the input level.
pub const TICK_INTERVAL: Duration = Duration::from_millis(100);

pub struct EngineDeps {
    pub recorder: Box<dyn Recorder>,
    pub pipeline: DictationPipeline,
    /// Reads the front app's selection when Edit selection starts.
    pub selection: Arc<dyn SelectionReader>,
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
            selection: deps.selection,
            running: None,
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
    selection: Arc<dyn SelectionReader>,
    /// The action being listened to, from its start until it is processed or dropped.
    running: Option<PreparedAction>,
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
                if let Some(action) = self.running_action() {
                    self.publish(Status::Listening {
                        action,
                        level: self.recorder.level(),
                    });
                }
            }
            None => {}
        }
    }

    fn running_action(&self) -> Option<Action> {
        self.running.as_ref().map(PreparedAction::action)
    }

    fn execute(&mut self, command: Command) {
        match command {
            Command::StartRecording(action) => self.start(action),
            Command::StopAndProcess => {
                let running = self.running.take();
                // Show Working before the microphone closes: closing it can take a moment
                // (Bluetooth headsets), and the indicator must stop counting right away.
                if let Some(prepared) = &running {
                    self.publish(Status::Working(prepared.action()));
                }
                let recording = self.recorder.stop();
                match running {
                    Some(prepared) => {
                        let outcome = self.pipeline.process(recording, &prepared, &self.settings);
                        // One action at a time: shortcuts pressed meanwhile are dropped.
                        while self.shortcuts.try_recv().is_ok() {}
                        self.publish(outcome.status);
                    }
                    None => {
                        // Cannot happen: listening always starts with a prepared action.
                        log::error!("stopped a recording that had no action; discarding it");
                        self.publish(Status::Idle);
                    }
                }
                self.controller
                    .handle(ControllerEvent::ProcessingFinished, Instant::now());
            }
            Command::Discard => {
                self.running = None;
                self.recorder.cancel();
                self.publish(Status::Idle);
            }
            Command::Cancel => {
                self.running = None;
                self.recorder.cancel();
                self.publish(Status::Cancelled);
            }
        }
    }

    /// Gets `action` ready (e.g. reads the selection) before anything is shown or the
    /// microphone opens; when it cannot run, it never starts.
    fn start(&mut self, action: Action) {
        let prepared = match PreparedAction::prepare(action, self.selection.as_ref()) {
            Ok(prepared) => prepared,
            Err(problem) => return self.abandon_start(problem),
        };
        self.running = Some(prepared);
        // Show the indicator before the microphone opens: opening it can take
        // a noticeable moment (first use, Bluetooth headsets).
        self.publish(Status::Listening { action, level: 0.0 });
        if let Err(problem) = self.recorder.start() {
            self.abandon_start(problem);
        }
    }

    /// The action never began: forget it entirely and tell the user why.
    fn abandon_start(&mut self, problem: Problem) {
        self.running = None;
        self.controller
            .handle(ControllerEvent::RecordingFailed, Instant::now());
        self.publish(Status::Problem(problem));
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
    use crate::fakes::{
        speech, FakeInserter, FakePolisher, FakeSelectionReader, FakeStore, FakeTranscriber,
    };
    use crate::rules::MAX_SELECTION_CHARS;
    use crate::{History, PolishError, Recording, TranscribeError};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use Action::{Dictate, DictateInEnglish, EditSelection};
    use ShortcutEvent::{ActionTapped, EscapePressed};

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
        start_delay: Duration,
        stop_delay: Duration,
        level: f32,
    }

    impl Recorder for FakeRecorder {
        fn start(&mut self) -> Result<(), Problem> {
            thread::sleep(self.start_delay);
            self.log.lock().unwrap().starts += 1;
            self.start_result.clone()
        }
        fn stop(&mut self) -> Recording {
            thread::sleep(self.stop_delay);
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
        ) -> Result<String, TranscribeError> {
            thread::sleep(self.0);
            Ok("hello team".into())
        }
    }

    struct Rig {
        handle: EngineHandle,
        recorder: Arc<Mutex<RecorderLog>>,
        selection: Arc<FakeSelectionReader>,
        inserter: Arc<FakeInserter>,
        store: Arc<FakeStore>,
        notified: Arc<AtomicUsize>,
    }

    struct Options {
        start_result: Result<(), Problem>,
        start_delay: Duration,
        stop_delay: Duration,
        transcriber: Arc<dyn crate::Transcriber>,
        answer: Result<&'static str, PolishError>,
        selection: Arc<FakeSelectionReader>,
    }

    impl Default for Options {
        fn default() -> Self {
            Self {
                start_result: Ok(()),
                start_delay: Duration::ZERO,
                stop_delay: Duration::ZERO,
                transcriber: FakeTranscriber::new(Ok("hello team")),
                answer: Ok("Hello team."),
                selection: FakeSelectionReader::new(Ok(Some("我們明天開會"))),
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
                FakePolisher::new(options.answer),
                inserter.clone(),
                store.clone(),
            );
            let counter = notified.clone();
            let handle = Engine::spawn(
                EngineDeps {
                    recorder: Box::new(FakeRecorder {
                        log: recorder.clone(),
                        start_result: options.start_result,
                        start_delay: options.start_delay,
                        stop_delay: options.stop_delay,
                        level: 0.42,
                    }),
                    pipeline,
                    selection: options.selection.clone(),
                    settings: Settings::default(),
                },
                Box::new(move || {
                    counter.fetch_add(1, Ordering::SeqCst);
                }),
            );
            Self {
                handle,
                recorder,
                selection: options.selection,
                inserter,
                store,
                notified,
            }
        }

        fn tap(&self, action: Action) {
            self.handle.shortcuts.send(ActionTapped(action)).unwrap();
        }

        fn escape(&self) {
            self.handle.shortcuts.send(EscapePressed).unwrap();
        }

        fn next_raw(&self) -> Status {
            self.handle.status.recv_timeout(WAIT).expect("status")
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

        /// The next status shows `action` listening.
        fn expect_listening(&self, action: Action) {
            match self.next_raw() {
                Status::Listening { action: shown, .. } => assert_eq!(shown, action),
                other => panic!("expected listening, got {other:?}"),
            }
        }

        /// Taps `action`, speaks long enough to count, taps again; returns the final status.
        fn run(&self, action: Action) -> Status {
            self.tap(action);
            self.expect_listening(action);
            hold(Duration::from_millis(550));
            self.tap(action);
            assert_eq!(self.next_status(), Status::Working(action));
            self.next_status()
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
    fn tapping_dictate_twice_records_processes_and_reports_done() {
        let rig = Rig::new(Options::default());

        assert_eq!(rig.run(Dictate), Status::Done);

        assert_eq!(rig.inserter.inserted(), ["Hello team."]);
        assert_eq!(rig.log(|l| (l.starts, l.stops, l.cancels)), (1, 1, 0));
        assert_eq!(
            rig.selection.calls(),
            0,
            "dictating never reads the selection"
        );
        assert_eq!(rig.store.saved()[0].entries()[0].action, Dictate);
    }

    #[test]
    fn dictate_in_english_is_carried_to_the_history_entry() {
        let rig = Rig::new(Options {
            transcriber: FakeTranscriber::new(Ok("我們開會")),
            answer: Ok("We have a meeting."),
            ..Options::default()
        });

        assert_eq!(rig.run(DictateInEnglish), Status::Done);

        assert_eq!(rig.inserter.inserted(), ["We have a meeting."]);
        let entry = rig.store.saved()[0].entries()[0].clone();
        assert_eq!(entry.action, DictateInEnglish);
        assert_eq!(entry.raw, "我們開會");
    }

    #[test]
    fn edit_selection_reads_the_selection_before_listening_and_replaces_it() {
        let rig = Rig::new(Options {
            transcriber: FakeTranscriber::new(Ok("translate to English")),
            answer: Ok("We have a meeting tomorrow."),
            ..Options::default()
        });

        assert_eq!(rig.run(EditSelection), Status::Done);

        assert_eq!(rig.selection.calls(), 1);
        assert_eq!(rig.inserter.inserted(), ["We have a meeting tomorrow."]);
        let entry = rig.store.saved()[0].entries()[0].clone();
        assert_eq!(entry.action, EditSelection);
        assert_eq!(entry.raw, "translate to English");
        assert_eq!(entry.original.as_deref(), Some("我們明天開會"));
    }

    #[test]
    fn nothing_selected_does_not_start_listening() {
        let rig = Rig::new(Options {
            selection: FakeSelectionReader::new(Ok(None)),
            ..Options::default()
        });

        rig.tap(EditSelection);

        assert_eq!(rig.next_raw(), Status::Problem(Problem::NoSelection));
        hold(TICK_INTERVAL * 3);
        let (rest, _, recorder) = rig.shut_down();
        assert!(rest.is_empty(), "never listened: {rest:?}");
        let log = recorder.lock().unwrap();
        assert_eq!((log.starts, log.stops, log.cancels), (0, 0, 0));
    }

    #[test]
    fn a_selection_over_the_limit_does_not_start_listening() {
        let too_long = "a".repeat(MAX_SELECTION_CHARS + 1);
        let rig = Rig::new(Options {
            selection: FakeSelectionReader::new(Ok(Some(&too_long))),
            ..Options::default()
        });

        rig.tap(EditSelection);

        assert_eq!(rig.next_raw(), Status::Problem(Problem::SelectionTooLong));
        assert_eq!(rig.log(|l| l.starts), 0);
        assert!(rig.inserter.inserted().is_empty());
        assert!(rig.store.saved().is_empty());
    }

    #[test]
    fn after_a_refused_start_a_new_attempt_is_allowed() {
        let rig = Rig::new(Options {
            selection: FakeSelectionReader::new(Ok(None)),
            ..Options::default()
        });

        rig.tap(EditSelection);
        assert_eq!(rig.next_raw(), Status::Problem(Problem::NoSelection));

        assert_eq!(rig.run(Dictate), Status::Done);
    }

    #[test]
    fn listening_is_shown_before_a_slow_microphone_finishes_opening() {
        let rig = Rig::new(Options {
            start_delay: Duration::from_millis(400),
            ..Options::default()
        });

        let pressed = Instant::now();
        rig.tap(Dictate);
        let first = rig.next_raw();

        assert_eq!(
            first,
            Status::Listening {
                action: Dictate,
                level: 0.0
            }
        );
        assert!(pressed.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn working_is_shown_before_a_slow_microphone_finishes_closing() {
        let rig = Rig::new(Options {
            stop_delay: Duration::from_millis(400),
            ..Options::default()
        });
        rig.tap(DictateInEnglish);
        rig.expect_listening(DictateInEnglish);
        hold(Duration::from_millis(550));

        let stopped = Instant::now();
        rig.tap(DictateInEnglish);

        assert_eq!(rig.next_status(), Status::Working(DictateInEnglish));
        assert!(stopped.elapsed() < Duration::from_millis(200));
        assert_eq!(rig.next_status(), Status::Done);
    }

    #[test]
    fn listening_publishes_the_action_and_live_level_on_each_tick() {
        let rig = Rig::new(Options::default());

        rig.tap(DictateInEnglish);
        rig.expect_listening(DictateInEnglish);

        // the next updates arrive from the clock, not from a shortcut
        for _ in 0..2 {
            assert_eq!(
                rig.next_raw(),
                Status::Listening {
                    action: DictateInEnglish,
                    level: 0.42
                }
            );
        }
    }

    #[test]
    fn another_actions_shortcut_is_ignored_while_one_is_listening() {
        let rig = Rig::new(Options::default());

        rig.tap(Dictate);
        rig.expect_listening(Dictate);
        rig.tap(DictateInEnglish);
        hold(Duration::from_millis(550));

        // still Dictate on the indicator, and only one recording was started
        rig.expect_listening(Dictate);
        assert_eq!(rig.log(|l| (l.starts, l.stops, l.cancels)), (1, 0, 0));
        rig.tap(Dictate);
        assert_eq!(rig.next_status(), Status::Working(Dictate));
    }

    #[test]
    fn stopping_within_half_a_second_is_discarded_back_to_idle() {
        let rig = Rig::new(Options::default());

        rig.tap(Dictate);
        rig.expect_listening(Dictate);
        rig.tap(Dictate);

        assert_eq!(rig.next_status(), Status::Idle);
        assert_eq!(rig.log(|l| (l.stops, l.cancels)), (0, 1));
        assert!(rig.inserter.inserted().is_empty());
    }

    #[test]
    fn escape_cancels_and_records_nothing() {
        let rig = Rig::new(Options::default());

        rig.tap(EditSelection);
        rig.expect_listening(EditSelection);
        rig.escape();

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

        rig.tap(Dictate);
        assert_eq!(
            rig.next_status(),
            Status::Problem(Problem::MicrophonePermission)
        );
        rig.tap(EditSelection);
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

        rig.tap(Dictate);
        assert_eq!(
            rig.next_raw(),
            Status::Listening {
                action: Dictate,
                level: 0.0
            }
        );
        assert_eq!(
            rig.next_raw(),
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

        rig.tap(Dictate);
        rig.expect_listening(Dictate);
        hold(Duration::from_millis(550));
        rig.tap(Dictate);
        assert_eq!(rig.next_status(), Status::Working(Dictate));
        rig.tap(EditSelection);
        rig.tap(Dictate);
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

        assert_eq!(rig.run(Dictate), Status::Done);

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

        shortcuts.send(ActionTapped(Dictate)).unwrap();
        assert!(matches!(
            status.recv_timeout(WAIT).unwrap(),
            Status::Listening { .. }
        ));
    }

    #[test]
    fn on_status_is_called_after_every_status() {
        let rig = Rig::new(Options::default());
        rig.tap(Dictate);
        rig.escape();
        let mut seen = 0;
        loop {
            seen += 1;
            if rig.next_raw() == Status::Cancelled {
                break;
            }
        }

        let (rest, notified, _) = rig.shut_down();

        assert_eq!(notified, seen + rest.len());
    }

    #[test]
    fn dropping_the_handle_stops_the_engine_and_releases_the_microphone() {
        let rig = Rig::new(Options::default());
        rig.tap(Dictate);
        rig.expect_listening(Dictate);

        let (_, _, recorder) = rig.shut_down();

        assert_eq!(recorder.lock().unwrap().cancels, 1);
    }

    #[test]
    fn unreachable_assistant_is_published_as_the_final_status() {
        let rig = Rig::new(Options {
            answer: Err(PolishError::Unreachable("down".into())),
            ..Options::default()
        });

        assert_eq!(
            rig.run(DictateInEnglish),
            Status::Problem(Problem::AssistantUnreachable(DictateInEnglish))
        );
    }
}
