//! In-process stand-ins for the real adapters, for running the UI on its own.
use super::{Permissions, PrivacyPane, RepaintHandle, Services};
use chrono::Local;
use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use voice_core::{
    EngineHandle, History, HistoryEntry, HistoryStore, Problem, Settings, SettingsStore,
    ShortcutEvent, Status,
};

/// How often the fake engine reports a new input level while listening.
const LEVEL_TICK: Duration = Duration::from_millis(60);
/// How long the fake engine pretends to work.
const WORK_TICKS: u32 = 12;
/// Sentences the fake engine "hears".
const SAMPLES: [(&str, &str); 3] = [
    (
        "um so the meeting is uh moved to three pm",
        "The meeting is moved to 3 PM.",
    ),
    (
        "remind me to buy milk and eggs",
        "Remind me to buy milk and eggs.",
    ),
    ("你好 这是一个测试", "你好，这是一个测试。"),
];

#[derive(Default)]
pub struct InMemorySettings(Mutex<Settings>);

impl SettingsStore for InMemorySettings {
    fn load(&self) -> Settings {
        self.0.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn save(&self, settings: &Settings) -> Result<(), String> {
        let mut slot = self.0.lock().map_err(|e| e.to_string())?;
        *slot = settings.clone();
        Ok(())
    }
}

#[derive(Default)]
pub struct InMemoryHistory(Mutex<History>);

impl HistoryStore for InMemoryHistory {
    fn load(&self) -> History {
        self.0.lock().map(|h| h.clone()).unwrap_or_default()
    }

    fn save(&self, history: &History) -> Result<(), String> {
        let mut slot = self.0.lock().map_err(|e| e.to_string())?;
        *slot = history.clone();
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Listening { ticks: u32 },
    Working { ticks_left: u32 },
}

/// A scripted dictation: shortcuts start and stop it, time moves it along.
#[derive(Debug)]
pub struct ScriptedDictation {
    phase: Phase,
    finished: usize,
}

/// What a step of the script produced.
#[derive(Debug, Default, PartialEq)]
pub struct Step {
    pub statuses: Vec<Status>,
    /// A finished dictation to add to history: `(raw, polished)`.
    pub entry: Option<(&'static str, &'static str)>,
}

impl ScriptedDictation {
    pub fn new() -> Self {
        Self {
            phase: Phase::Idle,
            finished: 0,
        }
    }

    /// Whether time-driven steps are pending (the engine loop then ticks quickly).
    pub fn is_busy(&self) -> bool {
        self.phase != Phase::Idle
    }

    pub fn on_shortcut(&mut self, event: ShortcutEvent) -> Step {
        match (self.phase, event) {
            (Phase::Idle, ShortcutEvent::PushToTalkDown | ShortcutEvent::HandsFreePressed) => {
                self.phase = Phase::Listening { ticks: 0 };
                Step {
                    statuses: vec![Status::Listening { level: level_at(0) }],
                    entry: None,
                }
            }
            (
                Phase::Listening { .. },
                ShortcutEvent::PushToTalkUp | ShortcutEvent::HandsFreePressed,
            ) => {
                self.phase = Phase::Working {
                    ticks_left: WORK_TICKS,
                };
                Step {
                    statuses: vec![Status::Working],
                    entry: None,
                }
            }
            (Phase::Listening { .. }, ShortcutEvent::EscapePressed) => {
                self.phase = Phase::Idle;
                Step {
                    statuses: vec![Status::Cancelled],
                    entry: None,
                }
            }
            _ => Step::default(),
        }
    }

    pub fn on_tick(&mut self) -> Step {
        match self.phase {
            Phase::Idle => Step::default(),
            Phase::Listening { ticks } => {
                let ticks = ticks + 1;
                self.phase = Phase::Listening { ticks };
                Step {
                    statuses: vec![Status::Listening {
                        level: level_at(ticks),
                    }],
                    entry: None,
                }
            }
            Phase::Working { ticks_left } if ticks_left > 1 => {
                self.phase = Phase::Working {
                    ticks_left: ticks_left - 1,
                };
                Step::default()
            }
            Phase::Working { .. } => {
                self.phase = Phase::Idle;
                self.finished += 1;
                // Every fourth dictation shows a problem so the message can be seen.
                if self.finished.is_multiple_of(4) {
                    Step {
                        statuses: vec![Status::Problem(Problem::AssistantUnreachable)],
                        entry: None,
                    }
                } else {
                    Step {
                        statuses: vec![Status::Done],
                        entry: Some(SAMPLES[self.finished % SAMPLES.len()]),
                    }
                }
            }
        }
    }
}

/// A pretend microphone level that rises and falls.
fn level_at(ticks: u32) -> f32 {
    let t = ticks as f32 * 0.35;
    (0.5 + 0.5 * t.sin()) * (0.6 + 0.4 * (t * 0.37).cos().abs())
}

fn record(history: &dyn HistoryStore, (raw, polished): (&str, &str)) {
    let mut all = history.load();
    all.record(HistoryEntry {
        at: Local::now(),
        raw: raw.to_string(),
        polished: polished.to_string(),
    });
    if let Err(e) = history.save(&all) {
        log::warn!("fake engine could not save history: {e}");
    }
}

/// Applies one step: publishes statuses, records history, wakes the UI.
fn publish(
    step: Step,
    history: &dyn HistoryStore,
    status: &Sender<Status>,
    repaint: &RepaintHandle,
) {
    if let Some(entry) = step.entry {
        record(history, entry);
    }
    for s in step.statuses {
        if status.send(s).is_err() {
            return;
        }
    }
    repaint.request();
}

/// Runs the scripted dictation until both input channels close.
fn run_engine(
    shortcuts: Receiver<ShortcutEvent>,
    settings: Receiver<Settings>,
    status: Sender<Status>,
    history: Arc<dyn HistoryStore>,
    repaint: RepaintHandle,
) {
    let mut script = ScriptedDictation::new();
    loop {
        let wait = if script.is_busy() {
            LEVEL_TICK
        } else {
            Duration::from_secs(3600)
        };
        match shortcuts.recv_timeout(wait) {
            Ok(event) => publish(script.on_shortcut(event), &*history, &status, &repaint),
            Err(RecvTimeoutError::Timeout) => {
                publish(script.on_tick(), &*history, &status, &repaint)
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while let Ok(new) = settings.try_recv() {
            log::info!(
                "fake engine received settings: cleanup={}",
                new.cleanup_enabled
            );
        }
    }
}

fn spawn_engine(history: Arc<dyn HistoryStore>, repaint: RepaintHandle) -> EngineHandle {
    let (shortcuts_tx, shortcuts_rx) = unbounded();
    let (settings_tx, settings_rx) = unbounded();
    let (status_tx, status_rx) = unbounded();
    std::thread::spawn(move || run_engine(shortcuts_rx, settings_rx, status_tx, history, repaint));
    EngineHandle {
        shortcuts: shortcuts_tx,
        settings: settings_tx,
        status: status_rx,
    }
}

pub fn fake_permissions() -> Permissions {
    let accessibility = Arc::new(AtomicBool::new(false));
    let granted = accessibility.clone();
    Permissions {
        microphone_granted: Arc::new(|| true),
        accessibility_granted: Arc::new(move || granted.load(Ordering::SeqCst)),
        prompt_accessibility: Arc::new(move || accessibility.store(true, Ordering::SeqCst)),
        open_privacy_settings: Arc::new(|pane: PrivacyPane| {
            log::info!("fake: would open the {pane:?} privacy pane");
        }),
    }
}

pub fn fake_default_model_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join("Library/Application Support/VoiceAssistant/models/ggml-small.bin")
}

pub fn fake_download(
    _path: &std::path::Path,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(), String> {
    const TOTAL: u64 = 100 * 1024 * 1024;
    for step in 1..=20u64 {
        std::thread::sleep(Duration::from_millis(60));
        progress(TOTAL / 20 * step, Some(TOTAL));
    }
    Ok(())
}

pub fn build() -> Services {
    let settings_store: Arc<dyn SettingsStore> = Arc::new(InMemorySettings::default());
    let history_store: Arc<dyn HistoryStore> = Arc::new(InMemoryHistory::default());
    let repaint = RepaintHandle::default();
    Services {
        engine: spawn_engine(history_store.clone(), repaint.clone()),
        settings_store,
        history_store,
        repaint,
        list_input_devices: Arc::new(|| {
            vec![
                "MacBook Pro Microphone".to_string(),
                "USB Headset".to_string(),
            ]
        }),
        list_models: Arc::new(|_url| {
            Ok(vec![
                "qwen2.5-7b-instruct".to_string(),
                "llama-3.2-3b-instruct".to_string(),
            ])
        }),
        default_model_path: Arc::new(fake_default_model_path),
        download_model: Arc::new(fake_download),
        permissions: fake_permissions(),
        update_shortcuts: Arc::new(|s: &Settings| {
            log::info!(
                "fake: shortcuts now {:?} / {:?}",
                s.push_to_talk,
                s.hands_free
            );
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn statuses(step: Step) -> Vec<Status> {
        step.statuses
    }

    fn listening() -> ScriptedDictation {
        let mut s = ScriptedDictation::new();
        s.on_shortcut(ShortcutEvent::PushToTalkDown);
        s
    }

    fn finish(s: &mut ScriptedDictation) -> Step {
        s.on_shortcut(ShortcutEvent::PushToTalkUp);
        let mut last = Step::default();
        while s.is_busy() {
            last = s.on_tick();
        }
        last
    }

    #[test]
    fn pressing_starts_listening() {
        let mut s = ScriptedDictation::new();
        assert!(!s.is_busy());
        let step = s.on_shortcut(ShortcutEvent::PushToTalkDown);
        assert!(matches!(statuses(step)[..], [Status::Listening { .. }]));
        assert!(s.is_busy());
    }

    #[test]
    fn hands_free_toggles() {
        let mut s = ScriptedDictation::new();
        s.on_shortcut(ShortcutEvent::HandsFreePressed);
        let step = s.on_shortcut(ShortcutEvent::HandsFreePressed);
        assert_eq!(statuses(step), [Status::Working]);
    }

    #[test]
    fn listening_ticks_report_levels_in_range() {
        let mut s = listening();
        for _ in 0..50 {
            match &statuses(s.on_tick())[..] {
                [Status::Listening { level }] => assert!((0.0..=1.0).contains(level)),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn release_works_then_finishes_with_an_entry() {
        let mut s = listening();
        let step = finish(&mut s);
        assert_eq!(step.statuses, [Status::Done]);
        assert!(step.entry.is_some());
    }

    #[test]
    fn every_fourth_dictation_is_a_problem_without_entry() {
        let mut s = ScriptedDictation::new();
        let mut last = Step::default();
        for _ in 0..4 {
            s.on_shortcut(ShortcutEvent::PushToTalkDown);
            last = finish(&mut s);
        }
        assert_eq!(
            last.statuses,
            [Status::Problem(Problem::AssistantUnreachable)]
        );
        assert_eq!(last.entry, None);
    }

    #[test]
    fn escape_cancels_only_while_listening() {
        let mut s = listening();
        assert_eq!(
            statuses(s.on_shortcut(ShortcutEvent::EscapePressed)),
            [Status::Cancelled]
        );
        assert!(!s.is_busy());
        assert_eq!(s.on_shortcut(ShortcutEvent::EscapePressed), Step::default());
    }

    #[test]
    fn irrelevant_events_do_nothing() {
        let mut s = ScriptedDictation::new();
        assert_eq!(s.on_shortcut(ShortcutEvent::PushToTalkUp), Step::default());
        assert_eq!(s.on_tick(), Step::default());
        s.on_shortcut(ShortcutEvent::PushToTalkUp);
        s.on_shortcut(ShortcutEvent::PushToTalkDown);
        let mut w = listening();
        w.on_shortcut(ShortcutEvent::PushToTalkUp);
        assert_eq!(
            w.on_shortcut(ShortcutEvent::PushToTalkDown),
            Step::default()
        );
    }

    #[test]
    fn in_memory_stores_round_trip() {
        let settings = InMemorySettings::default();
        let changed = Settings {
            cleanup_enabled: false,
            ..Settings::default()
        };
        assert_eq!(settings.load(), Settings::default());
        settings.save(&changed).unwrap();
        assert_eq!(settings.load(), changed);

        let history = InMemoryHistory::default();
        assert!(history.load().entries().is_empty());
        record(&history, ("raw", "polished"));
        assert_eq!(history.load().entries()[0].polished, "polished");
    }

    #[test]
    fn engine_thread_runs_a_dictation_and_records_history() {
        let history: Arc<dyn HistoryStore> = Arc::new(InMemoryHistory::default());
        let handle = spawn_engine(history.clone(), RepaintHandle::default());
        handle
            .shortcuts
            .send(ShortcutEvent::PushToTalkDown)
            .unwrap();
        handle.shortcuts.send(ShortcutEvent::PushToTalkUp).unwrap();
        handle.settings.send(Settings::default()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = handle
                .status
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("a Done status");
            if status == Status::Done {
                break;
            }
        }
        assert_eq!(history.load().entries().len(), 1);
    }

    #[test]
    fn engine_thread_ends_when_the_ui_goes_away() {
        let history: Arc<dyn HistoryStore> = Arc::new(InMemoryHistory::default());
        let (tx, rx) = unbounded();
        let (_stx, srx) = unbounded();
        let (status_tx, status_rx) = unbounded();
        drop(tx);
        run_engine(rx, srx, status_tx, history, RepaintHandle::default());
        drop(status_rx);
    }

    #[test]
    fn publishing_to_a_gone_ui_does_not_panic() {
        let history = InMemoryHistory::default();
        let (tx, rx) = unbounded();
        drop(rx);
        publish(
            Step {
                statuses: vec![Status::Done, Status::Working],
                entry: None,
            },
            &history,
            &tx,
            &RepaintHandle::default(),
        );
    }

    #[test]
    fn a_history_that_cannot_save_does_not_stop_the_fake_engine() {
        struct Broken;
        impl HistoryStore for Broken {
            fn load(&self) -> History {
                History::default()
            }
            fn save(&self, _: &History) -> Result<(), String> {
                Err("read-only".into())
            }
        }
        record(&Broken, ("a", "b"));
    }

    #[test]
    fn fake_permissions_flip_after_prompt() {
        let p = fake_permissions();
        assert!((p.microphone_granted)());
        assert!(!(p.accessibility_granted)());
        (p.prompt_accessibility)();
        assert!((p.accessibility_granted)());
        (p.open_privacy_settings)(PrivacyPane::Microphone);
    }

    #[test]
    fn fake_download_reports_growing_progress() {
        let mut seen = Vec::new();
        fake_download(std::path::Path::new("/x"), &mut |d, t| seen.push((d, t))).unwrap();
        assert_eq!(seen.len(), 20);
        assert_eq!(seen.last().unwrap().0, seen.last().unwrap().1.unwrap());
    }

    #[test]
    fn default_model_path_is_under_application_support() {
        assert!(fake_default_model_path().ends_with("VoiceAssistant/models/ggml-small.bin"));
    }

    #[test]
    fn built_services_answer() {
        let s = build();
        assert_eq!((s.list_input_devices)().len(), 2);
        assert_eq!((s.list_models)("http://x").unwrap().len(), 2);
        assert_eq!(s.settings_store.load(), Settings::default());
        (s.update_shortcuts)(&Settings::default());
        assert!((s.default_model_path)().ends_with("ggml-small.bin"));
    }
}
