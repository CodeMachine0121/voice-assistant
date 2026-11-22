//! What the buttons do, apart from drawing them.
use crate::wiring::Services;
use voice_core::Settings;

/// Persists `settings`, hands them to the engine and updates the shortcut listener.
/// Nothing is applied when saving fails.
pub fn save_settings(services: &Services, settings: &Settings) -> Result<(), String> {
    services.settings_store.save(settings)?;
    services
        .engine
        .settings
        .send(settings.clone())
        .map_err(|_| "The dictation engine is not running.".to_string())?;
    (services.apply_settings)(settings);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        save_settings(&services, &changed()).unwrap();
        assert_eq!(services.settings_store.load(), changed());
        assert_eq!(probe.engine_settings(), vec![changed()]);
        assert_eq!(probe.shortcut_updates(), vec![changed()]);
    }

    #[test]
    fn a_failed_save_applies_nothing() {
        let (services, probe) = test_services();
        probe.fail_saving("disk full");
        let err = save_settings(&services, &changed()).unwrap_err();
        assert_eq!(err, "disk full");
        assert!(probe.engine_settings().is_empty());
        assert!(probe.shortcut_updates().is_empty());
    }

    #[test]
    fn a_stopped_engine_is_reported() {
        let (mut services, probe) = test_services();
        let (tx, rx) = crossbeam_channel::unbounded();
        drop(rx);
        services.engine.settings = tx;
        let err = save_settings(&services, &changed()).unwrap_err();
        assert!(err.contains("not running"));
        assert!(probe.shortcut_updates().is_empty());
    }
}
