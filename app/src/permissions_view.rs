//! Rows for the permission section of the Settings window.
use crate::wiring::{Permissions, PrivacyPane};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRow {
    pub name: &'static str,
    pub why: &'static str,
    /// `None` = macOS gives no way to ask; the user checks in System Settings.
    pub granted: Option<bool>,
    pub pane: PrivacyPane,
}

impl PermissionRow {
    pub fn status_text(&self) -> &'static str {
        match self.granted {
            Some(true) => "Granted",
            Some(false) => "Not granted",
            None => "Check in System Settings",
        }
    }
}

pub fn rows(permissions: &Permissions) -> Vec<PermissionRow> {
    vec![
        PermissionRow {
            name: "Microphone",
            why: "to hear you",
            granted: Some((permissions.microphone_granted)()),
            pane: PrivacyPane::Microphone,
        },
        PermissionRow {
            name: "Accessibility",
            why: "to type the text into other apps",
            granted: Some((permissions.accessibility_granted)()),
            pane: PrivacyPane::Accessibility,
        },
        PermissionRow {
            name: "Input Monitoring",
            why: "to notice your shortcuts",
            granted: None,
            pane: PrivacyPane::InputMonitoring,
        },
    ]
}

/// What the "Open System Settings" button of `row` does: for Accessibility, macOS's
/// own prompt first (it lists the app), then the pane.
pub fn open_settings(permissions: &Permissions, row: &PermissionRow) {
    if row.pane == PrivacyPane::Accessibility && row.granted != Some(true) {
        (permissions.prompt_accessibility)();
    }
    (permissions.open_privacy_settings)(row.pane);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn permissions(mic: bool, ax: bool, log: Arc<Mutex<Vec<String>>>) -> Permissions {
        let prompt_log = log.clone();
        Permissions {
            microphone_granted: Arc::new(move || mic),
            accessibility_granted: Arc::new(move || ax),
            prompt_accessibility: Arc::new(move || {
                prompt_log.lock().unwrap().push("prompt".into())
            }),
            open_privacy_settings: Arc::new(move |pane| {
                log.lock().unwrap().push(format!("open {pane:?}"))
            }),
        }
    }

    #[test]
    fn rows_reflect_what_macos_says() {
        let log = Arc::default();
        let rows = rows(&permissions(true, false, log));
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name, "Microphone");
        assert_eq!(rows[0].granted, Some(true));
        assert_eq!(rows[0].pane, PrivacyPane::Microphone);
        assert_eq!(rows[1].name, "Accessibility");
        assert_eq!(rows[1].granted, Some(false));
        assert_eq!(rows[2].name, "Input Monitoring");
        assert_eq!(rows[2].granted, None);
        assert_eq!(rows[2].pane, PrivacyPane::InputMonitoring);
    }

    #[test]
    fn status_texts() {
        let row = |granted| PermissionRow {
            name: "x",
            why: "y",
            granted,
            pane: PrivacyPane::Microphone,
        };
        assert_eq!(row(Some(true)).status_text(), "Granted");
        assert_eq!(row(Some(false)).status_text(), "Not granted");
        assert_eq!(row(None).status_text(), "Check in System Settings");
    }

    #[test]
    fn missing_accessibility_prompts_then_opens_the_pane() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let p = permissions(true, false, log.clone());
        open_settings(&p, &rows(&p)[1]);
        assert_eq!(*log.lock().unwrap(), ["prompt", "open Accessibility"]);
    }

    #[test]
    fn other_rows_only_open_the_pane() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let p = permissions(false, true, log.clone());
        let rows = rows(&p);
        open_settings(&p, &rows[0]);
        open_settings(&p, &rows[1]);
        open_settings(&p, &rows[2]);
        assert_eq!(
            *log.lock().unwrap(),
            [
                "open Microphone",
                "open Accessibility",
                "open InputMonitoring"
            ]
        );
    }
}
