//! Services for unit tests: in-memory stores and recorders of what was asked of them.
use super::{fakes, settings_saver, Permissions, RepaintHandle, Services};
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::sync::{Arc, Mutex};
use voice_core::{EngineHandle, HistoryStore, Settings, SettingsStore, Status};
use voice_stt::{OpenAiKey, SecretStore};

/// A settings store that can be told to fail.
#[derive(Default)]
struct FlakySettings {
    inner: Mutex<Settings>,
    failure: Mutex<Option<String>>,
}

impl SettingsStore for FlakySettings {
    fn load(&self) -> Settings {
        self.inner.lock().unwrap().clone()
    }

    fn save(&self, settings: &Settings) -> Result<(), String> {
        if let Some(e) = self.failure.lock().unwrap().clone() {
            return Err(e);
        }
        *self.inner.lock().unwrap() = settings.clone();
        Ok(())
    }
}

/// A secret store that can be told to fail, standing in for the Keychain.
#[derive(Default)]
struct FlakySecret {
    secret: Mutex<Option<String>>,
    failure: Mutex<Option<String>>,
    reads: Mutex<usize>,
}

impl FlakySecret {
    fn failure(&self) -> Result<(), String> {
        match self.failure.lock().unwrap().clone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl SecretStore for FlakySecret {
    fn read(&self) -> Result<Option<String>, String> {
        *self.reads.lock().unwrap() += 1;
        Ok(self.secret.lock().unwrap().clone())
    }

    fn save(&self, secret: &str) -> Result<(), String> {
        self.failure()?;
        *self.secret.lock().unwrap() = Some(secret.to_string());
        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        self.failure()?;
        *self.secret.lock().unwrap() = None;
        Ok(())
    }
}

/// Sees what the app asked of the services.
pub struct Probe {
    settings: Arc<FlakySettings>,
    engine_settings: Receiver<Settings>,
    status_tx: Sender<Status>,
    applied: Arc<Mutex<Vec<Settings>>>,
    secret: Arc<FlakySecret>,
}

impl Probe {
    /// What the secret store (the Keychain in the real app) holds.
    pub fn saved_key(&self) -> Option<String> {
        self.secret.secret.lock().unwrap().clone()
    }

    /// How often the secret store has been read so far.
    pub fn key_reads(&self) -> usize {
        *self.secret.reads.lock().unwrap()
    }

    pub fn fail_key_storage(&self, message: &str) {
        *self.secret.failure.lock().unwrap() = Some(message.to_string());
    }

    pub fn fail_saving(&self, message: &str) {
        *self.settings.failure.lock().unwrap() = Some(message.to_string());
    }

    /// Publishes `status` as the engine would.
    pub fn send_status(&self, status: Status) {
        self.status_tx.send(status).unwrap();
    }

    pub fn engine_settings(&self) -> Vec<Settings> {
        self.engine_settings.try_iter().collect()
    }

    /// Settings handed to the adapters, in order.
    pub fn applied_settings(&self) -> Vec<Settings> {
        self.applied.lock().unwrap().clone()
    }
}

pub fn test_services() -> (Services, Probe) {
    let settings = Arc::new(FlakySettings::default());
    let history: Arc<dyn HistoryStore> = Arc::new(fakes::InMemoryHistory::default());
    let (shortcuts_tx, _shortcuts_rx) = unbounded();
    let (settings_tx, settings_rx) = unbounded();
    let (status_tx, status_rx) = unbounded::<Status>();
    let updates = Arc::new(Mutex::new(Vec::new()));
    let recorded = updates.clone();
    let secret = Arc::new(FlakySecret::default());
    let services = Services {
        settings_store: settings.clone(),
        history_store: history,
        engine: EngineHandle {
            shortcuts: shortcuts_tx,
            settings: settings_tx.clone(),
            status: status_rx,
        },
        repaint: RepaintHandle::default(),
        list_input_devices: Arc::new(|| vec!["Mic A".to_string()]),
        list_models: Arc::new(|_| Ok(vec!["model-a".to_string()])),
        speech_key: Arc::new(OpenAiKey::with_environment(secret.clone(), || None)),
        permissions: Permissions {
            microphone_granted: Arc::new(|| Some(true)),
            request_microphone: Arc::new(|| {}),
            accessibility_granted: Arc::new(|| true),
            prompt_accessibility: Arc::new(|| {}),
            shortcuts_listening: Arc::new(|| true),
            open_privacy_settings: Arc::new(|_| {}),
        },
        save_settings: settings_saver(settings.clone(), settings_tx, move |s: &Settings| {
            recorded.lock().unwrap().push(s.clone())
        }),
    };
    (
        services,
        Probe {
            settings,
            engine_settings: settings_rx,
            status_tx,
            applied: updates,
            secret,
        },
    )
}
