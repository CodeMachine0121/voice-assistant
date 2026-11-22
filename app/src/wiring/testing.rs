//! Services for unit tests: in-memory stores and recorders of what was asked of them.
use super::{fakes, Permissions, RepaintHandle, Services};
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::sync::{Arc, Mutex};
use voice_core::{EngineHandle, HistoryStore, Settings, SettingsStore, Status};

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

/// Sees what the app asked of the services.
pub struct Probe {
    settings: Arc<FlakySettings>,
    engine_settings: Receiver<Settings>,
    status_tx: Sender<Status>,
    shortcut_updates: Arc<Mutex<Vec<Settings>>>,
}

impl Probe {
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

    pub fn shortcut_updates(&self) -> Vec<Settings> {
        self.shortcut_updates.lock().unwrap().clone()
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
    let services = Services {
        settings_store: settings.clone(),
        history_store: history,
        engine: EngineHandle {
            shortcuts: shortcuts_tx,
            settings: settings_tx,
            status: status_rx,
        },
        repaint: RepaintHandle::default(),
        list_input_devices: Arc::new(|| vec!["Mic A".to_string()]),
        list_models: Arc::new(|_| Ok(vec!["model-a".to_string()])),
        default_model_path: Arc::new(|| "/default/model.bin".into()),
        download_model: Arc::new(|_, _| Ok(())),
        permissions: Permissions {
            microphone_granted: Arc::new(|| true),
            accessibility_granted: Arc::new(|| true),
            prompt_accessibility: Arc::new(|| {}),
            open_privacy_settings: Arc::new(|_| {}),
        },
        apply_settings: Arc::new(move |s: &Settings| recorded.lock().unwrap().push(s.clone())),
    };
    (
        services,
        Probe {
            settings,
            engine_settings: settings_rx,
            status_tx,
            shortcut_updates: updates,
        },
    )
}
