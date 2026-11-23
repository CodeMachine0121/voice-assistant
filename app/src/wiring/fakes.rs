//! Demo adapters: the real dictation engine with a pretend microphone, speech provider
//! and writing assistant, for running the app without devices, network or permissions.
use super::{Permissions, PrivacyPane, RepaintHandle, Services};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use voice_core::{
    rules, DictationPipeline, Engine, EngineDeps, EngineHandle, History, HistoryStore,
    PersonalDictionary, PolishError, Polisher, Problem, Recorder, Recording, SelectionReader,
    Settings, SettingsStore, TextInserter, TranscribeError, Transcriber, WritingTask,
};
use voice_stt::{MemorySecretStore, OpenAiKey};

/// How long the demo speech model and writing assistant each pretend to work.
const DEMO_WORK: Duration = Duration::from_millis(350);
/// Sentences the demo "hears": the heard words, the cleaned-up form and the English form.
const SAMPLES: [(&str, &str, &str); 3] = [
    (
        "um so the meeting is uh moved to three pm",
        "The meeting is moved to 3 PM.",
        "The meeting is moved to 3 PM.",
    ),
    (
        "remind me to buy milk and eggs",
        "Remind me to buy milk and eggs.",
        "Remind me to buy milk and eggs.",
    ),
    (
        "你好 这是一个测试",
        "你好，這是一個測試。",
        "Hello, this is a test.",
    ),
];

/// What the demo "has selected" when Edit starts.
const DEMO_SELECTION: &str = "This is the demo selection.";

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

/// A pretend microphone level that rises and falls.
fn level_at(elapsed: Duration) -> f32 {
    let t = elapsed.as_secs_f32() * 6.0;
    (0.5 + 0.5 * t.sin()) * (0.6 + 0.4 * (t * 0.37).cos().abs())
}

/// A microphone that "hears" a steady voiced tone for as long as it records, so the
/// real speech detection and length rules apply.
#[derive(Default)]
pub struct DemoRecorder {
    started: Option<Instant>,
}

impl Recorder for DemoRecorder {
    fn start(&mut self) -> Result<(), Problem> {
        self.started = Some(Instant::now());
        Ok(())
    }

    fn stop(&mut self) -> Recording {
        let elapsed = self.started.take().map(|t| t.elapsed()).unwrap_or_default();
        let n = (elapsed.as_secs_f64() * rules::SAMPLE_RATE as f64) as usize;
        Recording::new((0..n).map(|i| 0.2 * (i as f32 * 0.07).sin()).collect())
    }

    fn cancel(&mut self) {
        self.started = None;
    }

    fn level(&self) -> f32 {
        self.started.map_or(0.0, |t| level_at(t.elapsed()))
    }
}

/// Speech recognition and writing assistant that cycle through `SAMPLES`. Every
/// fourth cleanup reports the assistant as unreachable so that message can be seen.
#[derive(Default)]
pub struct DemoSpeech {
    heard: AtomicUsize,
    polished: AtomicUsize,
}

impl Transcriber for DemoSpeech {
    fn transcribe(&self, _: &Recording, _: &PersonalDictionary) -> Result<String, TranscribeError> {
        std::thread::sleep(DEMO_WORK);
        let i = self.heard.fetch_add(1, Ordering::SeqCst);
        Ok(SAMPLES[i % SAMPLES.len()].0.to_string())
    }
}

impl Polisher for DemoSpeech {
    fn polish(
        &self,
        task: &WritingTask<'_>,
        _: &PersonalDictionary,
    ) -> Result<String, PolishError> {
        std::thread::sleep(DEMO_WORK);
        let i = self.polished.fetch_add(1, Ordering::SeqCst) + 1;
        if i.is_multiple_of(4) {
            return Err(PolishError::Unreachable("demo".into()));
        }
        let sample = |heard: &str| SAMPLES.iter().find(|(h, _, _)| *h == heard);
        match task {
            WritingTask::CleanUp { heard } => sample(heard).map(|(_, clean, _)| clean.to_string()),
            WritingTask::TranslateToEnglish { heard } => {
                sample(heard).map(|(_, _, english)| english.to_string())
            }
            WritingTask::EditSelection {
                instruction,
                selection,
            } => Some(format!("{selection} (edited: {instruction})")),
        }
        .ok_or_else(|| PolishError::InvalidResponse("unknown demo sentence".into()))
    }
}

/// Always "has selected" the same sentence.
pub struct DemoSelection;

impl SelectionReader for DemoSelection {
    fn selected_text(&self) -> Result<Option<String>, Problem> {
        Ok(Some(DEMO_SELECTION.to_string()))
    }
}

/// Logs the text instead of typing it into another app.
pub struct LogInserter;

impl TextInserter for LogInserter {
    fn insert(&self, text: &str) -> Result<(), Problem> {
        log::info!("demo: would insert {text:?}");
        Ok(())
    }
}

/// The real dictation engine, running on demo adapters.
fn spawn_engine(history: Arc<dyn HistoryStore>, repaint: RepaintHandle) -> EngineHandle {
    let speech = Arc::new(DemoSpeech::default());
    let pipeline = DictationPipeline::new(speech.clone(), speech, Arc::new(LogInserter), history);
    Engine::spawn(
        EngineDeps {
            recorder: Box::new(DemoRecorder::default()),
            pipeline,
            selection: Arc::new(DemoSelection),
            settings: Settings::default(),
        },
        Box::new(move || repaint.request()),
    )
}

pub fn fake_permissions() -> Permissions {
    let accessibility = Arc::new(AtomicBool::new(false));
    let granted = accessibility.clone();
    Permissions {
        microphone_granted: Arc::new(|| Some(true)),
        request_microphone: Arc::new(|| log::info!("fake: would ask for the microphone")),
        shortcuts_listening: Arc::new(|| true),
        accessibility_granted: Arc::new(move || granted.load(Ordering::SeqCst)),
        prompt_accessibility: Arc::new(move || accessibility.store(true, Ordering::SeqCst)),
        open_privacy_settings: Arc::new(|pane: PrivacyPane| {
            log::info!("fake: would open the {pane:?} privacy pane");
        }),
    }
}

pub fn build() -> Services {
    let settings_store: Arc<dyn SettingsStore> = Arc::new(InMemorySettings::default());
    let history_store: Arc<dyn HistoryStore> = Arc::new(InMemoryHistory::default());
    let repaint = RepaintHandle::default();
    let engine = spawn_engine(history_store.clone(), repaint.clone());
    // Demo mode never talks to OpenAI, so the key is only kept in memory.
    let speech_key = Arc::new(OpenAiKey::with_environment(
        Arc::new(MemorySecretStore::default()),
        || None,
    ));
    Services {
        save_settings: super::settings_saver(
            settings_store.clone(),
            speech_key.clone(),
            engine.settings.clone(),
            |s| {
                log::info!("demo: shortcuts now {:?}", s.shortcuts);
            },
        ),
        engine,
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
        speech_key,
        permissions: fake_permissions(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voice_core::{speech::contains_speech, Action, ShortcutEvent, Status};

    #[test]
    fn in_memory_stores_round_trip() {
        let settings = InMemorySettings::default();
        let changed = Settings {
            cleanup_enabled: false,
            ..Settings::default()
        };
        settings.save(&changed).unwrap();
        assert_eq!(settings.load(), changed);
        let history = InMemoryHistory::default();
        assert!(history.load().entries().is_empty());
        history.save(&History::default()).unwrap();
    }

    #[test]
    fn demo_microphone_hears_speech_for_as_long_as_it_records() {
        let mut mic = DemoRecorder::default();
        assert_eq!(mic.level(), 0.0);
        mic.start().unwrap();
        std::thread::sleep(Duration::from_millis(400));
        let level = mic.level();
        assert!((0.0..=1.0).contains(&level));
        let rec = mic.stop();
        assert!(rec.samples.len() >= (rules::SAMPLE_RATE as usize * 4 / 10));
        assert!(contains_speech(&rec));
        assert_eq!(mic.level(), 0.0);
        mic.start().unwrap();
        mic.cancel();
        assert_eq!(mic.stop(), Recording::default());
    }

    #[test]
    fn demo_speech_cycles_sentences_and_sometimes_loses_the_assistant() {
        let speech = DemoSpeech::default();
        let none = PersonalDictionary::default();
        let mut outcomes = Vec::new();
        for _ in 0..4 {
            let heard = speech.transcribe(&Recording::default(), &none).unwrap();
            outcomes.push(speech.polish(&WritingTask::CleanUp { heard: &heard }, &none));
        }
        assert_eq!(outcomes[0], Ok(SAMPLES[0].1.to_string()));
        assert_eq!(outcomes[1], Ok(SAMPLES[1].1.to_string()));
        assert!(matches!(outcomes[3], Err(PolishError::Unreachable(_))));
        assert!(matches!(
            speech.polish(
                &WritingTask::CleanUp {
                    heard: "never said"
                },
                &none
            ),
            Err(PolishError::InvalidResponse(_))
        ));
    }

    #[test]
    fn demo_assistant_translates_and_edits() {
        let speech = DemoSpeech::default();
        let none = PersonalDictionary::default();
        let translated = speech.polish(
            &WritingTask::TranslateToEnglish {
                heard: SAMPLES[2].0,
            },
            &none,
        );
        assert_eq!(translated, Ok(SAMPLES[2].2.to_string()));
        let edited = speech.polish(
            &WritingTask::EditSelection {
                instruction: "shorten",
                selection: "long text",
            },
            &none,
        );
        assert_eq!(edited, Ok("long text (edited: shorten)".to_string()));
    }

    #[test]
    fn demo_selection_is_always_available() {
        assert_eq!(
            DemoSelection.selected_text(),
            Ok(Some(DEMO_SELECTION.to_string()))
        );
    }

    #[test]
    fn log_inserter_always_succeeds() {
        assert_eq!(LogInserter.insert("hello"), Ok(()));
    }

    #[test]
    fn demo_engine_runs_a_real_dictation_into_history() {
        let history: Arc<dyn HistoryStore> = Arc::new(InMemoryHistory::default());
        let handle = spawn_engine(history.clone(), RepaintHandle::default());
        let tap = ShortcutEvent::ActionTapped(Action::Dictate);
        handle.shortcuts.send(tap).unwrap();
        std::thread::sleep(rules::MIN_DICTATION + Duration::from_millis(100));
        handle.shortcuts.send(tap).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = handle
                .status
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("a Done status");
            if status == Status::Done {
                break;
            }
        }
        let entries = history.load().entries().to_vec();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].polished, SAMPLES[0].1);
        assert_eq!(entries[0].action, Action::Dictate);
    }

    #[test]
    fn fake_permissions_flip_after_prompt() {
        let p = fake_permissions();
        assert_eq!((p.microphone_granted)(), Some(true));
        assert!(!(p.accessibility_granted)());
        (p.prompt_accessibility)();
        assert!((p.accessibility_granted)());
        (p.open_privacy_settings)(PrivacyPane::Microphone);
    }

    #[test]
    fn built_services_answer() {
        let s = build();
        assert_eq!((s.list_input_devices)().len(), 2);
        assert_eq!((s.list_models)("http://x").unwrap().len(), 2);
        assert_eq!(s.settings_store.load(), Settings::default());
        (s.save_settings)(&super::super::SettingsChange {
            settings: Settings::default(),
            new_key: None,
        })
        .unwrap();
        assert_eq!(s.speech_key.status(), voice_stt::KeyStatus::Missing);
    }
}
