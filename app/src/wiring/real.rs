//! The production composition: real microphone, speech model, writing assistant,
//! macOS shortcuts/paste/permissions and JSON storage.
//!
//! Several adapters depend on settings the user can change while the app runs
//! (microphone, speech model file, assistant address and model, shortcuts).
//! Each is wrapped in a small "configurable" adapter that `apply` reconfigures.
use super::{settings_saver, Permissions, PrivacyPane, RepaintHandle, Services};
use crossbeam_channel::{unbounded, Sender};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use voice_audio::CpalRecorder;
use voice_core::{
    DictationPipeline, Engine, EngineDeps, HistoryStore, PersonalDictionary, PolishError, Polisher,
    Problem, Recorder, Recording, Settings, SettingsStore, Transcriber,
};
use voice_macos::{EventTapShortcuts, MacPermissions, PasteInserter, ShortcutConfig};
use voice_polish::LmStudioPolisher;
use voice_storage::{JsonHistoryStore, JsonSettingsStore};
use voice_stt::WhisperTranscriber;

/// Message shown when no speech model file exists yet.
pub const SPEECH_MODEL_MISSING: &str =
    "Speech model missing – download it in Settings › Speech model";

/// Where the speech model lives for these settings.
pub fn speech_model_path(settings: &Settings) -> PathBuf {
    settings
        .speech_model_path
        .clone()
        .unwrap_or_else(voice_stt::default_model_path)
}

/// Loads a speech model from a path.
type ModelLoader = dyn Fn(&std::path::Path) -> Result<Arc<dyn Transcriber>, String> + Send + Sync;

/// Speech recognition that follows the configured model file and loads it lazily,
/// so the app starts even before the model has been downloaded.
pub struct ConfigurableTranscriber {
    path: Mutex<PathBuf>,
    loaded: Mutex<Option<(PathBuf, Arc<dyn Transcriber>)>>,
    loader: Box<ModelLoader>,
}

impl ConfigurableTranscriber {
    pub fn new(path: PathBuf, loader: Box<ModelLoader>) -> Self {
        Self {
            path: Mutex::new(path),
            loaded: Mutex::new(None),
            loader,
        }
    }

    pub fn set_path(&self, path: PathBuf) {
        *lock(&self.path) = path;
    }

    fn current(&self) -> Result<Arc<dyn Transcriber>, String> {
        let path = lock(&self.path).clone();
        let mut loaded = lock(&self.loaded);
        if let Some((loaded_path, model)) = loaded.as_ref() {
            if *loaded_path == path {
                return Ok(model.clone());
            }
        }
        if !path.exists() {
            *loaded = None;
            return Err(SPEECH_MODEL_MISSING.to_string());
        }
        let model = (self.loader)(&path)?;
        *loaded = Some((path, model.clone()));
        Ok(model)
    }
}

impl Transcriber for ConfigurableTranscriber {
    fn transcribe(
        &self,
        recording: &Recording,
        dictionary: &PersonalDictionary,
    ) -> Result<String, String> {
        self.current()?.transcribe(recording, dictionary)
    }
}

/// The writing assistant at whatever address and model the settings name.
pub struct ConfigurablePolisher(RwLock<Arc<dyn Polisher>>);

impl ConfigurablePolisher {
    pub fn new(polisher: Arc<dyn Polisher>) -> Self {
        Self(RwLock::new(polisher))
    }

    pub fn replace(&self, polisher: Arc<dyn Polisher>) {
        match self.0.write() {
            Ok(mut guard) => *guard = polisher,
            Err(poisoned) => *poisoned.into_inner() = polisher,
        }
    }

    fn current(&self) -> Arc<dyn Polisher> {
        match self.0.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl Polisher for ConfigurablePolisher {
    fn polish(&self, raw: &str, dictionary: &PersonalDictionary) -> Result<String, PolishError> {
        self.current().polish(raw, dictionary)
    }
}

/// A recorder the engine owns while the app keeps a handle to change the microphone.
pub struct SharedRecorder<R>(Arc<Mutex<R>>);

impl<R> Clone for SharedRecorder<R> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<R> SharedRecorder<R> {
    pub fn new(recorder: R) -> Self {
        Self(Arc::new(Mutex::new(recorder)))
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut R) -> T) -> T {
        f(&mut lock(&self.0))
    }
}

impl<R: Recorder> Recorder for SharedRecorder<R> {
    fn start(&mut self) -> Result<(), Problem> {
        self.with(|r| r.start())
    }
    fn stop(&mut self) -> Recording {
        self.with(|r| r.stop())
    }
    fn cancel(&mut self) {
        self.with(|r| r.cancel())
    }
    fn level(&self) -> f32 {
        lock(&self.0).level()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn shortcut_config(settings: &Settings) -> ShortcutConfig {
    ShortcutConfig {
        push_to_talk: settings.push_to_talk.clone(),
        hands_free: settings.hands_free.clone(),
    }
}

fn polisher_for(settings: &Settings) -> Arc<dyn Polisher> {
    Arc::new(LmStudioPolisher::new(
        settings.assistant_url.clone(),
        settings.assistant_model.clone(),
    ))
}

/// Runs the global shortcut listener on its own thread (the listener is tied to the
/// thread that created it) and returns a sender for new shortcut settings.
/// When macOS refuses the listener, the user is sent to the Input Monitoring pane
/// and the next settings change retries.
fn spawn_shortcut_listener(
    initial: ShortcutConfig,
    sink: Sender<voice_core::ShortcutEvent>,
) -> Sender<ShortcutConfig> {
    let (tx, rx) = unbounded::<ShortcutConfig>();
    std::thread::Builder::new()
        .name("shortcut-listener".into())
        .spawn(move || {
            let start =
                |config: ShortcutConfig| match EventTapShortcuts::start(config, sink.clone()) {
                    Ok(listener) => Some(listener),
                    Err(problem) => {
                        log::warn!("global shortcuts unavailable: {problem}");
                        MacPermissions::prompt_accessibility();
                        None
                    }
                };
            let mut listener = start(initial);
            for config in rx {
                match &listener {
                    Some(l) => l.update(config),
                    None => listener = start(config),
                }
            }
        })
        .map_err(|e| log::error!("cannot start the shortcut listener thread: {e}"))
        .ok();
    tx
}

fn mac_permissions() -> Permissions {
    Permissions {
        microphone_granted: Arc::new(MacPermissions::microphone_granted),
        request_microphone: Arc::new(MacPermissions::request_microphone),
        // Replaced by the shortcut listener's own state once it reports it.
        shortcuts_listening: Arc::new(|| false),
        accessibility_granted: Arc::new(MacPermissions::accessibility_granted),
        prompt_accessibility: Arc::new(|| {
            MacPermissions::prompt_accessibility();
        }),
        open_privacy_settings: Arc::new(|pane: PrivacyPane| {
            let pane = match pane {
                PrivacyPane::Microphone => voice_macos::PrivacyPane::Microphone,
                PrivacyPane::Accessibility => voice_macos::PrivacyPane::Accessibility,
                PrivacyPane::InputMonitoring => voice_macos::PrivacyPane::InputMonitoring,
            };
            if let Err(e) = MacPermissions::open_privacy_settings(pane) {
                log::warn!("cannot open System Settings: {e}");
            }
        }),
    }
}

pub fn build() -> Services {
    let settings_store: Arc<dyn SettingsStore> = Arc::new(JsonSettingsStore::in_default_location());
    let history_store: Arc<dyn HistoryStore> = Arc::new(JsonHistoryStore::in_default_location());
    let settings = settings_store.load();
    let repaint = RepaintHandle::default();

    let transcriber = Arc::new(ConfigurableTranscriber::new(
        speech_model_path(&settings),
        Box::new(|path| {
            WhisperTranscriber::new(path.to_path_buf()).map(|t| Arc::new(t) as Arc<dyn Transcriber>)
        }),
    ));
    let polisher = Arc::new(ConfigurablePolisher::new(polisher_for(&settings)));
    let recorder = SharedRecorder::new(CpalRecorder::new(settings.microphone.clone()));

    let pipeline = DictationPipeline::new(
        transcriber.clone(),
        polisher.clone(),
        Arc::new(PasteInserter::new()),
        history_store.clone(),
    );
    let wake = repaint.clone();
    let engine = Engine::spawn(
        EngineDeps {
            recorder: Box::new(recorder.clone()),
            pipeline,
            settings: settings.clone(),
        },
        Box::new(move || wake.request()),
    );
    let shortcuts = spawn_shortcut_listener(shortcut_config(&settings), engine.shortcuts.clone());

    let save_settings = settings_saver(
        settings_store.clone(),
        engine.settings.clone(),
        move |s: &Settings| {
            transcriber.set_path(speech_model_path(s));
            polisher.replace(polisher_for(s));
            recorder.with(|r| r.set_device(s.microphone.clone()));
            let _ = shortcuts.send(shortcut_config(s));
        },
    );
    Services {
        save_settings,
        settings_store,
        history_store,
        engine,
        repaint,
        list_input_devices: Arc::new(CpalRecorder::list_input_devices),
        list_models: Arc::new(voice_polish::list_models),
        default_model_path: Arc::new(voice_stt::default_model_path),
        download_model: Arc::new(|dest, progress| {
            voice_stt::download_default_model(dest, progress)
        }),
        permissions: mac_permissions(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Echo(&'static str);
    impl Transcriber for Echo {
        fn transcribe(&self, _: &Recording, _: &PersonalDictionary) -> Result<String, String> {
            Ok(self.0.to_string())
        }
    }
    impl Polisher for Echo {
        fn polish(&self, _: &str, _: &PersonalDictionary) -> Result<String, PolishError> {
            Ok(self.0.to_string())
        }
    }

    fn existing_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("va-real-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("model.bin");
        std::fs::write(&file, b"x").unwrap();
        file
    }

    fn counting_loader(count: Arc<AtomicUsize>) -> Box<ModelLoader> {
        Box::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(Echo("heard")) as Arc<dyn Transcriber>)
        })
    }

    #[test]
    fn missing_speech_model_tells_the_user_to_download_it() {
        let count = Arc::new(AtomicUsize::new(0));
        let t = ConfigurableTranscriber::new(
            PathBuf::from("/definitely/not/here.bin"),
            counting_loader(count.clone()),
        );
        let err = t
            .transcribe(&Recording::default(), &PersonalDictionary::default())
            .unwrap_err();
        assert_eq!(err, SPEECH_MODEL_MISSING);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn speech_model_is_loaded_once_and_reloaded_when_the_path_changes() {
        let count = Arc::new(AtomicUsize::new(0));
        let first = existing_file("a");
        let t = ConfigurableTranscriber::new(first.clone(), counting_loader(count.clone()));
        let none = PersonalDictionary::default();
        assert_eq!(t.transcribe(&Recording::default(), &none).unwrap(), "heard");
        t.transcribe(&Recording::default(), &none).unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);

        t.set_path(existing_file("b"));
        t.transcribe(&Recording::default(), &none).unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn speech_model_load_errors_are_reported() {
        let t = ConfigurableTranscriber::new(
            existing_file("c"),
            Box::new(|_| Err("not a model".to_string())),
        );
        let err = t
            .transcribe(&Recording::default(), &PersonalDictionary::default())
            .unwrap_err();
        assert_eq!(err, "not a model");
    }

    #[test]
    fn writing_assistant_follows_the_latest_settings() {
        let p = ConfigurablePolisher::new(Arc::new(Echo("old")));
        let none = PersonalDictionary::default();
        assert_eq!(p.polish("x", &none).unwrap(), "old");
        p.replace(Arc::new(Echo("new")));
        assert_eq!(p.polish("x", &none).unwrap(), "new");
    }

    #[test]
    fn default_speech_model_location_is_used_unless_one_is_chosen() {
        let mut s = Settings::default();
        assert_eq!(speech_model_path(&s), voice_stt::default_model_path());
        s.speech_model_path = Some(PathBuf::from("/models/custom.bin"));
        assert_eq!(speech_model_path(&s), PathBuf::from("/models/custom.bin"));
    }

    #[test]
    fn shortcut_settings_reach_the_listener_config() {
        let s = Settings {
            push_to_talk: voice_core::Shortcut::Fn,
            ..Settings::default()
        };
        let c = shortcut_config(&s);
        assert_eq!(c.push_to_talk, voice_core::Shortcut::Fn);
        assert_eq!(c.hands_free, s.hands_free);
    }

    #[derive(Default)]
    struct Mic {
        started: bool,
    }
    impl Recorder for Mic {
        fn start(&mut self) -> Result<(), Problem> {
            self.started = true;
            Ok(())
        }
        fn stop(&mut self) -> Recording {
            self.started = false;
            Recording::new(vec![0.5])
        }
        fn cancel(&mut self) {
            self.started = false;
        }
        fn level(&self) -> f32 {
            if self.started {
                1.0
            } else {
                0.0
            }
        }
    }

    #[test]
    fn shared_recorder_delegates_and_stays_reachable_from_the_app() {
        let app_side = SharedRecorder::new(Mic::default());
        let mut engine_side: Box<dyn Recorder> = Box::new(app_side.clone());
        engine_side.start().unwrap();
        assert_eq!(engine_side.level(), 1.0);
        assert!(app_side.with(|m| m.started));
        assert_eq!(engine_side.stop(), Recording::new(vec![0.5]));
        engine_side.start().unwrap();
        engine_side.cancel();
        assert_eq!(engine_side.level(), 0.0);
    }
}
