//! The production composition: real microphone, speech model, writing assistant,
//! macOS shortcuts/paste/permissions and JSON storage.
//!
//! Several adapters depend on settings the user can change while the app runs
//! (microphone, speech model, assistant address and model, shortcuts).
//! Each is wrapped in a small "configurable" adapter that `apply` reconfigures.
use super::{settings_saver, Permissions, PrivacyPane, RepaintHandle, Services};
use std::sync::{Arc, Mutex, RwLock};
use voice_audio::CpalRecorder;
use voice_core::{
    DictationPipeline, Engine, EngineDeps, HistoryStore, PersonalDictionary, PolishError, Polisher,
    Problem, Recorder, Recording, Settings, SettingsStore, SpeechProvider, SpeechSettings,
    TranscribeError, Transcriber, WritingTask,
};
use voice_macos::{
    ClipboardSelectionReader, KeychainSecret, MacPermissions, PasteInserter, ShortcutConfig,
    ShortcutHandle, ShortcutService,
};
use voice_polish::LmStudioPolisher;
use voice_storage::{JsonHistoryStore, JsonSettingsStore};
use voice_stt::{OpenAiKey, OpenAiTranscriber, SecretStore};

/// Builds the transcriber for a provider and speech model.
type TranscriberFactory = dyn Fn(&SpeechSettings) -> Arc<dyn Transcriber> + Send + Sync;

/// Where speech turns into words: the provider and speech model of
/// `Settings::speech`. `apply` switches it when settings are saved.
pub struct SpeechProviderSwitch {
    factory: Box<TranscriberFactory>,
    chosen: RwLock<Arc<dyn Transcriber>>,
}

impl SpeechProviderSwitch {
    pub fn new(settings: &SpeechSettings, factory: Box<TranscriberFactory>) -> Self {
        let chosen = RwLock::new(factory(settings));
        Self { factory, chosen }
    }

    /// Uses the provider and speech model of `settings` from the next dictation on.
    pub fn apply(&self, settings: &SpeechSettings) {
        let next = (self.factory)(settings);
        match self.chosen.write() {
            Ok(mut guard) => *guard = next,
            Err(poisoned) => *poisoned.into_inner() = next,
        }
    }

    fn current(&self) -> Arc<dyn Transcriber> {
        match self.chosen.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl Transcriber for SpeechProviderSwitch {
    fn transcribe(
        &self,
        recording: &Recording,
        dictionary: &PersonalDictionary,
    ) -> Result<String, TranscribeError> {
        self.current().transcribe(recording, dictionary)
    }
}

/// The production transcriber for each speech provider.
fn transcriber_for(settings: &SpeechSettings, key: &Arc<OpenAiKey>) -> Arc<dyn Transcriber> {
    match settings.provider {
        SpeechProvider::OpenAi => Arc::new(openai_transcriber(settings, key)),
    }
}

fn openai_transcriber(settings: &SpeechSettings, key: &Arc<OpenAiKey>) -> OpenAiTranscriber {
    OpenAiTranscriber::new(settings.model.clone(), key.clone())
}

/// The login Keychain as the place the OpenAI key is kept.
struct Keychain(KeychainSecret);

impl SecretStore for Keychain {
    fn read(&self) -> Result<Option<String>, String> {
        self.0.read()
    }

    fn save(&self, secret: &str) -> Result<(), String> {
        self.0.save(secret)
    }

    fn remove(&self) -> Result<(), String> {
        self.0.remove()
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
    fn polish(
        &self,
        task: &WritingTask<'_>,
        dictionary: &PersonalDictionary,
    ) -> Result<String, PolishError> {
        self.current().polish(task, dictionary)
    }
}

/// A recorder whose input device can be switched.
pub trait SwitchableDevice {
    fn set_device(&mut self, device: Option<String>);
}

impl SwitchableDevice for CpalRecorder {
    fn set_device(&mut self, device: Option<String>) {
        CpalRecorder::set_device(self, device);
    }
}

/// A recorder the engine owns while the app keeps a handle to change the microphone.
/// A device change is only noted and applied at the next start, so saving settings
/// never waits for a microphone that is still opening.
pub struct SharedRecorder<R> {
    recorder: Arc<Mutex<R>>,
    next_device: Arc<Mutex<Option<Option<String>>>>,
}

impl<R> Clone for SharedRecorder<R> {
    fn clone(&self) -> Self {
        Self {
            recorder: self.recorder.clone(),
            next_device: self.next_device.clone(),
        }
    }
}

impl<R> SharedRecorder<R> {
    pub fn new(recorder: R) -> Self {
        Self {
            recorder: Arc::new(Mutex::new(recorder)),
            next_device: Arc::new(Mutex::new(None)),
        }
    }

    /// Uses `device` from the next dictation on. Never blocks on the recorder.
    pub fn choose_device(&self, device: Option<String>) {
        *lock(&self.next_device) = Some(device);
    }
}

impl<R: Recorder + SwitchableDevice> Recorder for SharedRecorder<R> {
    fn start(&mut self) -> Result<(), Problem> {
        let next = lock(&self.next_device).take();
        let mut recorder = lock(&self.recorder);
        if let Some(device) = next {
            recorder.set_device(device);
        }
        recorder.start()
    }
    fn stop(&mut self) -> Recording {
        lock(&self.recorder).stop()
    }
    fn cancel(&mut self) {
        lock(&self.recorder).cancel()
    }
    fn level(&self) -> f32 {
        lock(&self.recorder).level()
    }
}

/// Asks macOS whether the microphone may be used: `Some(true)` allowed,
/// `Some(false)` denied, `None` not asked yet.
type MicrophoneCheck = dyn Fn() -> Option<bool> + Send + Sync;

/// Checks the microphone permission before every recording. A denied microphone
/// does not fail on macOS — it records silence — so without this check the user
/// would be told "no speech heard" instead of how to grant access.
pub struct PermissionCheckedRecorder<R> {
    inner: R,
    check: Box<MicrophoneCheck>,
    request: Box<dyn Fn() + Send + Sync>,
}

impl<R> PermissionCheckedRecorder<R> {
    pub fn new(
        inner: R,
        check: Box<MicrophoneCheck>,
        request: Box<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            inner,
            check,
            request,
        }
    }
}

impl<R: Recorder> Recorder for PermissionCheckedRecorder<R> {
    fn start(&mut self) -> Result<(), Problem> {
        match (self.check)() {
            Some(true) => self.inner.start(),
            Some(false) => Err(Problem::MicrophonePermission),
            None => {
                // First use: show macOS's own request; this dictation cannot proceed.
                (self.request)();
                Err(Problem::MicrophonePermission)
            }
        }
    }
    fn stop(&mut self) -> Recording {
        self.inner.stop()
    }
    fn cancel(&mut self) {
        self.inner.cancel()
    }
    fn level(&self) -> f32 {
        self.inner.level()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn polisher_for(settings: &Settings) -> Arc<dyn Polisher> {
    Arc::new(LmStudioPolisher::new(
        settings.assistant_url.clone(),
        settings.assistant_model.clone(),
    ))
}

/// Asks macOS for permissions; `shortcuts` tells whether the shortcut listener runs
/// (which needs Input Monitoring).
fn mac_permissions(shortcuts: Arc<ShortcutHandle>) -> Permissions {
    Permissions {
        microphone_granted: Arc::new(MacPermissions::microphone_granted),
        request_microphone: Arc::new(MacPermissions::request_microphone),
        shortcuts_listening: Arc::new(move || shortcuts.is_listening()),
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

    let speech_key = Arc::new(OpenAiKey::new(Arc::new(Keychain(
        KeychainSecret::openai_key(),
    ))));
    let factory_key = speech_key.clone();
    let transcriber = Arc::new(SpeechProviderSwitch::new(
        &settings.speech,
        Box::new(move |speech| transcriber_for(speech, &factory_key)),
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
            recorder: Box::new(PermissionCheckedRecorder::new(
                recorder.clone(),
                Box::new(MacPermissions::microphone_granted),
                Box::new(MacPermissions::request_microphone),
            )),
            pipeline,
            selection: Arc::new(ClipboardSelectionReader::new()),
            settings: settings.clone(),
        },
        Box::new(move || wake.request()),
    );
    // The service keeps retrying until macOS allows the listener, so granting the
    // permission later takes effect without a restart.
    let shortcuts = Arc::new(ShortcutService::spawn(
        ShortcutConfig::from(&settings),
        engine.shortcuts.clone(),
    ));
    if !MacPermissions::accessibility_granted() {
        // Pasting into other apps needs it; macOS's prompt also lists the app.
        MacPermissions::prompt_accessibility();
    }
    let listener = shortcuts.clone();

    let save_settings = settings_saver(
        settings_store.clone(),
        speech_key.clone(),
        engine.settings.clone(),
        move |s: &Settings| {
            transcriber.apply(&s.speech);
            polisher.replace(polisher_for(s));
            recorder.choose_device(s.microphone.clone());
            listener.apply(ShortcutConfig::from(s));
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
        speech_key,
        permissions: mac_permissions(shortcuts),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Echo(&'static str);
    impl Transcriber for Echo {
        fn transcribe(
            &self,
            _: &Recording,
            _: &PersonalDictionary,
        ) -> Result<String, TranscribeError> {
            Ok(self.0.to_string())
        }
    }
    impl Polisher for Echo {
        fn polish(
            &self,
            _: &WritingTask<'_>,
            _: &PersonalDictionary,
        ) -> Result<String, PolishError> {
            Ok(self.0.to_string())
        }
    }

    fn heard(t: &dyn Transcriber) -> Result<String, TranscribeError> {
        t.transcribe(&Recording::default(), &PersonalDictionary::default())
    }

    fn speech(model: &str) -> SpeechSettings {
        SpeechSettings {
            model: model.to_string(),
            ..SpeechSettings::default()
        }
    }

    /// A switch whose transcribers say which speech model they were built for.
    fn naming_switch(settings: &SpeechSettings) -> SpeechProviderSwitch {
        SpeechProviderSwitch::new(
            settings,
            Box::new(|speech| {
                let said: &'static str = Box::leak(speech.model.clone().into_boxed_str());
                Arc::new(Echo(said)) as Arc<dyn Transcriber>
            }),
        )
    }

    #[test]
    fn the_saved_speech_model_does_the_listening() {
        let s = naming_switch(&SpeechSettings::default());

        assert_eq!(heard(&s).unwrap(), "gpt-4o-transcribe");
    }

    #[test]
    fn a_newly_chosen_speech_model_is_used_from_the_next_dictation() {
        let s = naming_switch(&SpeechSettings::default());

        s.apply(&speech("gpt-4o-mini-transcribe"));

        assert_eq!(heard(&s).unwrap(), "gpt-4o-mini-transcribe");
    }

    #[test]
    fn speech_provider_failures_reach_the_pipeline_unchanged() {
        struct Down;
        impl Transcriber for Down {
            fn transcribe(
                &self,
                _: &Recording,
                _: &PersonalDictionary,
            ) -> Result<String, TranscribeError> {
                Err(TranscribeError::KeyRejected)
            }
        }
        let s = SpeechProviderSwitch::new(
            &SpeechSettings::default(),
            Box::new(|_| Arc::new(Down) as Arc<dyn Transcriber>),
        );

        assert_eq!(heard(&s), Err(TranscribeError::KeyRejected));
    }

    #[test]
    fn openai_listens_with_the_saved_speech_model() {
        let key = Arc::new(OpenAiKey::with_environment(
            Arc::new(voice_stt::MemorySecretStore::default()),
            || None,
        ));

        let shown = format!("{:?}", openai_transcriber(&speech("whisper-1"), &key));

        assert!(shown.contains("model: \"whisper-1\""), "{shown}");
    }

    #[test]
    fn openai_without_a_key_asks_for_one_before_sending_anything() {
        let key = Arc::new(OpenAiKey::with_environment(
            Arc::new(voice_stt::MemorySecretStore::default()),
            || None,
        ));
        let t = transcriber_for(&SpeechSettings::default(), &key);

        let out = t.transcribe(
            &Recording::new(vec![0.2; 16_000]),
            &PersonalDictionary::default(),
        );

        assert_eq!(out, Err(TranscribeError::KeyMissing));
    }

    #[test]
    fn writing_assistant_follows_the_latest_settings() {
        let p = ConfigurablePolisher::new(Arc::new(Echo("old")));
        let none = PersonalDictionary::default();
        let task = WritingTask::CleanUp { heard: "x" };
        assert_eq!(p.polish(&task, &none).unwrap(), "old");
        p.replace(Arc::new(Echo("new")));
        assert_eq!(p.polish(&task, &none).unwrap(), "new");
    }

    #[derive(Default)]
    struct Mic {
        started: bool,
        device: Option<String>,
    }

    impl SwitchableDevice for Mic {
        fn set_device(&mut self, device: Option<String>) {
            self.device = device;
        }
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

    fn checked(granted: Option<bool>) -> (PermissionCheckedRecorder<Mic>, Arc<AtomicUsize>) {
        let requests = Arc::new(AtomicUsize::new(0));
        let counted = requests.clone();
        let rec = PermissionCheckedRecorder::new(
            Mic::default(),
            Box::new(move || granted),
            Box::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }),
        );
        (rec, requests)
    }

    #[test]
    fn denied_microphone_is_reported_and_nothing_is_recorded() {
        let (mut rec, requests) = checked(Some(false));
        assert_eq!(rec.start(), Err(Problem::MicrophonePermission));
        assert_eq!(rec.level(), 0.0);
        assert_eq!(requests.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn undecided_microphone_asks_macos_and_does_not_record() {
        let (mut rec, requests) = checked(None);
        assert_eq!(rec.start(), Err(Problem::MicrophonePermission));
        assert_eq!(rec.level(), 0.0);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn granted_microphone_records_normally() {
        let (mut rec, requests) = checked(Some(true));
        rec.start().unwrap();
        assert_eq!(rec.level(), 1.0);
        assert_eq!(rec.stop(), Recording::new(vec![0.5]));
        rec.start().unwrap();
        rec.cancel();
        assert_eq!(rec.level(), 0.0);
        assert_eq!(requests.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn shared_recorder_delegates_to_the_engine_side() {
        let app_side = SharedRecorder::new(Mic::default());
        let mut engine_side: Box<dyn Recorder> = Box::new(app_side.clone());
        engine_side.start().unwrap();
        assert_eq!(engine_side.level(), 1.0);
        assert_eq!(engine_side.stop(), Recording::new(vec![0.5]));
        engine_side.start().unwrap();
        engine_side.cancel();
        assert_eq!(engine_side.level(), 0.0);
    }

    #[test]
    fn device_change_applies_at_the_next_start_without_waiting_for_the_recorder() {
        let app_side = SharedRecorder::new(Mic::default());
        let mut engine_side = app_side.clone();
        let busy = lock(&app_side.recorder); // a microphone still opening
        app_side.choose_device(Some("USB Headset".into())); // must not block
        drop(busy);
        assert_eq!(lock(&app_side.recorder).device, None);
        engine_side.start().unwrap();
        assert_eq!(
            lock(&app_side.recorder).device.as_deref(),
            Some("USB Headset")
        );
        engine_side.cancel();
        lock(&app_side.recorder).device = Some("Other".into());
        engine_side.start().unwrap();
        assert_eq!(lock(&app_side.recorder).device.as_deref(), Some("Other"));
    }
}
