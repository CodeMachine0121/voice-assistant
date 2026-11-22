//! Composition root: the only place that knows which adapters back the ports.
//!
//! The rest of the app talks to `Services` and nothing else. `real` wires the
//! production adapters; `fakes` backs a demo mode (`VOICE_ASSISTANT_DEMO=1`) that
//! runs without microphone, speech model, writing assistant or permissions.
mod fakes;
mod real;
#[cfg(test)]
pub mod testing;

use crossbeam_channel::Sender;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use voice_core::{EngineHandle, HistoryStore, Settings, SettingsStore};

/// Names of the audio input devices, for the microphone picker.
pub type DeviceListFn = Arc<dyn Fn() -> Vec<String> + Send + Sync>;
/// Models offered by the writing assistant at a base URL.
pub type ModelListFn = Arc<dyn Fn(&str) -> Result<Vec<String>, String> + Send + Sync>;
/// Downloads the default speech model to a path, reporting `(downloaded, total)` bytes.
pub type ModelDownloadFn =
    Arc<dyn Fn(&Path, &mut dyn FnMut(u64, Option<u64>)) -> Result<(), String> + Send + Sync>;
/// Saves settings and puts them into effect everywhere; nothing changes on failure.
pub type SettingsSaveFn = Arc<dyn Fn(&Settings) -> Result<(), String> + Send + Sync>;

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
    pub default_model_path: Arc<dyn Fn() -> PathBuf + Send + Sync>,
    pub download_model: ModelDownloadFn,
    pub permissions: Permissions,
    pub save_settings: SettingsSaveFn,
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

/// The one way settings change: persist them, then hand them to the engine and to
/// the adapters that depend on them (`apply`). Nothing is applied when persisting
/// fails.
fn settings_saver(
    store: Arc<dyn SettingsStore>,
    engine: Sender<Settings>,
    apply: impl Fn(&Settings) + Send + Sync + 'static,
) -> SettingsSaveFn {
    Arc::new(move |settings: &Settings| {
        store.save(settings)?;
        engine
            .send(settings.clone())
            .map_err(|_| "The dictation engine is not running.".to_string())?;
        apply(settings);
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
    use voice_core::Shortcut;

    fn changed() -> Settings {
        Settings {
            push_to_talk: Shortcut::Fn,
            ..Settings::default()
        }
    }

    #[test]
    fn saving_persists_and_applies() {
        let (services, probe) = test_services();
        (services.save_settings)(&changed()).unwrap();
        assert_eq!(services.settings_store.load(), changed());
        assert_eq!(probe.engine_settings(), vec![changed()]);
        assert_eq!(probe.applied_settings(), vec![changed()]);
    }

    #[test]
    fn a_failed_save_applies_nothing() {
        let (services, probe) = test_services();
        probe.fail_saving("disk full");
        let err = (services.save_settings)(&changed()).unwrap_err();
        assert_eq!(err, "disk full");
        assert!(probe.engine_settings().is_empty());
        assert!(probe.applied_settings().is_empty());
    }

    #[test]
    fn a_stopped_engine_is_reported() {
        let store: Arc<dyn SettingsStore> = Arc::new(fakes::InMemorySettings::default());
        let (tx, rx) = crossbeam_channel::unbounded();
        drop(rx);
        let applied = Arc::new(std::sync::Mutex::new(0));
        let count = applied.clone();
        let save = settings_saver(store, tx, move |_| *count.lock().unwrap() += 1);
        let err = save(&changed()).unwrap_err();
        assert!(err.contains("not running"));
        assert_eq!(*applied.lock().unwrap(), 0);
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
