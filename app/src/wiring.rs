//! Composition root: the only place that knows which adapters back the ports.
//!
//! The rest of the app talks to `Services` and nothing else. `real` wires the
//! production adapters; `fakes` backs a demo mode (`VOICE_ASSISTANT_DEMO=1`) that
//! runs without microphone, speech model, writing assistant or permissions.
mod fakes;
mod real;
#[cfg(test)]
pub mod testing;

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
/// Hands saved settings to the adapters that depend on them (shortcuts, microphone,
/// speech model, writing assistant).
pub type SettingsApplyFn = Arc<dyn Fn(&Settings) + Send + Sync>;

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
    pub microphone_granted: Arc<dyn Fn() -> bool + Send + Sync>,
    pub accessibility_granted: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Asks macOS to show its accessibility prompt.
    pub prompt_accessibility: Arc<dyn Fn() + Send + Sync>,
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
    pub apply_settings: SettingsApplyFn,
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
