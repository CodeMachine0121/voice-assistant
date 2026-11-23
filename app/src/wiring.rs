//! Composition root: the only place that knows which adapters back the ports.
//!
//! The rest of the app talks to `Services` and nothing else. `real` wires the
//! production adapters; `fakes` backs a demo mode (`VOICE_ASSISTANT_DEMO=1`) that
//! runs without microphone, speech provider, writing assistant or permissions.
mod fakes;
mod real;
mod swappable;
#[cfg(test)]
pub mod testing;

use crossbeam_channel::Sender;
use eframe::egui;
use std::sync::{Arc, OnceLock};
use voice_core::{EngineHandle, HistoryStore, Settings, SettingsStore};
use voice_stt::{OpenAiKey, PreviousKey};

/// Names of the audio input devices, for the microphone picker.
pub type DeviceListFn = Arc<dyn Fn() -> Vec<String> + Send + Sync>;
/// Models offered by the writing assistant at a base URL.
pub type ModelListFn = Arc<dyn Fn(&str) -> Result<Vec<String>, String> + Send + Sync>;
/// What the Settings window asks to save: the settings, and a newly typed OpenAI key.
#[derive(Clone, PartialEq)]
pub struct SettingsChange {
    pub settings: Settings,
    /// `None` = keep the saved key.
    pub new_key: Option<String>,
}

impl std::fmt::Debug for SettingsChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsChange")
            .field("settings", &self.settings)
            .field("new_key", &self.new_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Why a change could not be saved. Either way nothing of it is in effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// The OpenAI key could not be kept.
    Key(String),
    /// The settings could not be kept or put into effect.
    Settings(String),
}

/// Saves a change and puts it into effect everywhere; nothing changes on failure.
pub type SettingsSaveFn = Arc<dyn Fn(&SettingsChange) -> Result<(), SaveError> + Send + Sync>;

/// Panes of System Settings › Privacy & Security the app can send the user to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyPane {
    Microphone,
    Accessibility,
    InputMonitoring,
}

/// What the app may ask of macOS about permissions.
#[derive(Clone)]
pub struct Permissions {
    /// `None` = the user has not been asked yet. Must not ask by itself.
    pub microphone_granted: Arc<dyn Fn() -> Option<bool> + Send + Sync>,
    /// Shows macOS's microphone request (only when not asked yet).
    pub request_microphone: Arc<dyn Fn() + Send + Sync>,
    pub accessibility_granted: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Asks macOS to show its accessibility prompt.
    pub prompt_accessibility: Arc<dyn Fn() + Send + Sync>,
    /// Whether the global shortcut listener is running (it needs Input Monitoring).
    pub shortcuts_listening: Arc<dyn Fn() -> bool + Send + Sync>,
    pub open_privacy_settings: Arc<dyn Fn(PrivacyPane) + Send + Sync>,
}

/// Wakes the UI from other threads. Empty until the window system is up.
#[derive(Clone, Default)]
pub struct RepaintHandle(Arc<OnceLock<egui::Context>>);

impl RepaintHandle {
    /// Connects the handle to the running UI. Only the first call counts.
    pub fn connect(&self, ctx: egui::Context) {
        let _ = self.0.set(ctx);
    }

    /// Asks the UI to run a frame (a no-op before `connect`).
    pub fn request(&self) {
        if let Some(ctx) = self.0.get() {
            ctx.request_repaint();
        }
    }
}

/// Everything the UI needs from the outside world.
#[derive(Clone)]
pub struct Services {
    pub settings_store: Arc<dyn SettingsStore>,
    pub history_store: Arc<dyn HistoryStore>,
    pub engine: EngineHandle,
    pub repaint: RepaintHandle,
    pub list_input_devices: DeviceListFn,
    pub list_models: ModelListFn,
    /// The OpenAI key the speech provider authenticates with.
    pub speech_key: Arc<OpenAiKey>,
    pub permissions: Permissions,
    pub save_settings: SettingsSaveFn,
    pub app_focus: AppFocus,
}

/// Moves keyboard focus between this app and the app the user works in. The app
/// lives in the menu bar only, and macOS does neither by itself. Called on the UI
/// thread.
#[derive(Clone)]
pub struct AppFocus {
    /// Makes the app the active app so its windows can come to the front.
    pub bring_to_front: Arc<dyn Fn() + Send + Sync>,
    /// Gives focus back to the app that was active before `bring_to_front`, if it
    /// is still running. Never hides this app: its menu must keep working.
    pub hand_back: Arc<dyn Fn() + Send + Sync>,
}

impl AppFocus {
    /// The real thing, through AppKit. Starts following which other app the user
    /// is in, so build it on the main thread at startup.
    pub fn macos() -> Self {
        voice_macos::track_focus();
        Self {
            bring_to_front: Arc::new(voice_macos::activate_app),
            hand_back: Arc::new(voice_macos::return_focus),
        }
    }
}

/// Environment variable that switches the app to demo mode.
pub const DEMO_ENV: &str = "VOICE_ASSISTANT_DEMO";

/// Builds the services the app runs on.
pub fn build_services() -> Services {
    if demo_mode(std::env::var(DEMO_ENV).ok().as_deref()) {
        log::info!("demo mode: using simulated dictation");
        fakes::build()
    } else {
        real::build()
    }
}

/// The one way settings change: keep the new key, persist the settings, then hand them
/// to the engine and to the adapters that depend on them (`apply`). The key goes first;
/// when it cannot be kept nothing else is saved. When the settings cannot be persisted,
/// or the engine cannot take them, the settings on disk and the key are put back the way
/// they were, so a change is never half applied.
fn settings_saver(
    store: Arc<dyn SettingsStore>,
    key: Arc<OpenAiKey>,
    engine: Sender<Settings>,
    apply: impl Fn(&Settings) + Send + Sync + 'static,
) -> SettingsSaveFn {
    Arc::new(move |change: &SettingsChange| {
        let previous = match &change.new_key {
            Some(new_key) => key.replace(new_key).map_err(SaveError::Key)?,
            None => PreviousKey::Unchanged,
        };
        let before = store.load();
        let kept = store.save(&change.settings).and_then(|()| {
            engine.send(change.settings.clone()).map_err(|_| {
                if let Err(undo) = store.save(&before) {
                    log::warn!("cannot put the previous settings back: {undo}");
                }
                "The dictation engine is not running.".to_string()
            })
        });
        if let Err(e) = kept {
            if let Err(undo) = previous.restore(&key) {
                log::warn!("cannot put the previous OpenAI key back: {undo}");
            }
            return Err(SaveError::Settings(e));
        }
        apply(&change.settings);
        Ok(())
    })
}

fn demo_mode(value: Option<&str>) -> bool {
    matches!(value, Some(v) if v == "1" || v.eq_ignore_ascii_case("true"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requesting_before_connecting_is_harmless() {
        RepaintHandle::default().request();
    }

    #[test]
    fn connected_handle_wakes_the_ui_and_first_connection_wins() {
        let handle = RepaintHandle::default();
        let first = egui::Context::default();
        handle.connect(first.clone());
        handle.connect(egui::Context::default());
        handle.request();
        assert!(first.has_requested_repaint());
    }

    use crate::wiring::testing::test_services;

    fn changed() -> Settings {
        Settings {
            microphone: Some("USB Mic".into()),
            ..Settings::default()
        }
    }

    fn change(new_key: Option<&str>) -> SettingsChange {
        SettingsChange {
            settings: changed(),
            new_key: new_key.map(str::to_string),
        }
    }

    #[test]
    fn saving_persists_and_applies() {
        let (services, probe) = test_services();
        (services.save_settings)(&change(None)).unwrap();
        assert_eq!(services.settings_store.load(), changed());
        assert_eq!(probe.engine_settings(), vec![changed()]);
        assert_eq!(probe.applied_settings(), vec![changed()]);
    }

    #[test]
    fn a_typed_key_is_kept_together_with_the_settings() {
        let (services, probe) = test_services();

        (services.save_settings)(&change(Some("sk-new"))).unwrap();

        assert_eq!(probe.saved_key().as_deref(), Some("sk-new"));
        assert_eq!(services.settings_store.load(), changed());
        assert_eq!(probe.applied_settings(), vec![changed()]);
    }

    #[test]
    fn a_key_that_cannot_be_kept_saves_and_applies_nothing() {
        let (services, probe) = test_services();
        probe.fail_key_storage("keychain locked");

        let err = (services.save_settings)(&change(Some("sk-new"))).unwrap_err();

        assert_eq!(err, SaveError::Key("keychain locked".into()));
        assert_eq!(services.settings_store.load(), Settings::default());
        assert!(probe.engine_settings().is_empty());
        assert!(probe.applied_settings().is_empty());
    }

    #[test]
    fn a_failed_save_applies_nothing_and_puts_the_previous_key_back() {
        let (services, probe) = test_services();
        services.speech_key.save("sk-old").unwrap();
        probe.fail_saving("disk full");

        let err = (services.save_settings)(&change(Some("sk-new"))).unwrap_err();

        assert_eq!(err, SaveError::Settings("disk full".into()));
        assert_eq!(probe.saved_key().as_deref(), Some("sk-old"));
        assert!(probe.engine_settings().is_empty());
        assert!(probe.applied_settings().is_empty());
    }

    #[test]
    fn a_failed_save_removes_a_first_key_again() {
        let (services, probe) = test_services();
        probe.fail_saving("disk full");

        let err = (services.save_settings)(&change(Some("sk-new"))).unwrap_err();

        assert_eq!(err, SaveError::Settings("disk full".into()));
        assert_eq!(probe.saved_key(), None);
    }

    #[test]
    fn a_stopped_engine_is_reported_and_the_settings_and_key_are_put_back() {
        let store: Arc<dyn SettingsStore> = Arc::new(fakes::InMemorySettings::default());
        let earlier = Settings {
            cleanup_enabled: false,
            ..Settings::default()
        };
        store.save(&earlier).unwrap();
        let secrets = Arc::new(voice_stt::MemorySecretStore::holding("sk-old"));
        let key = Arc::new(OpenAiKey::with_environment(secrets.clone(), || None));
        let (tx, rx) = crossbeam_channel::unbounded();
        drop(rx);
        let applied = Arc::new(std::sync::Mutex::new(0));
        let count = applied.clone();
        let save = settings_saver(store.clone(), key, tx, move |_| *count.lock().unwrap() += 1);

        let err = save(&change(Some("sk-new"))).unwrap_err();

        assert!(
            matches!(&err, SaveError::Settings(e) if e.contains("not running")),
            "{err:?}"
        );
        assert_eq!(*applied.lock().unwrap(), 0);
        assert_eq!(store.load(), earlier, "the settings on disk are put back");
        use voice_stt::SecretStore;
        assert_eq!(secrets.read().unwrap().as_deref(), Some("sk-old"));
    }

    #[test]
    fn a_change_never_shows_the_typed_key() {
        let shown = format!("{:?}", change(Some("sk-secret")));

        assert!(!shown.contains("sk-secret"), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
    }

    #[test]
    fn demo_mode_only_when_explicitly_enabled() {
        assert!(demo_mode(Some("1")));
        assert!(demo_mode(Some("TRUE")));
        assert!(!demo_mode(Some("0")));
        assert!(!demo_mode(Some("")));
        assert!(!demo_mode(None));
    }

    #[test]
    fn demo_services_give_a_working_engine_handle() {
        let services = fakes::build();
        assert!(services.engine.status.try_recv().is_err());
    }
}
