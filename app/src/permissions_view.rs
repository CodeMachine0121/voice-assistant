//! Rows for the permission section of the Settings window.
use crate::wiring::{Permissions, PrivacyPane};
use std::time::{Duration, Instant};

/// How often the Settings window asks macOS again while it is open.
pub const RECHECK_EVERY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Granted,
    Denied,
    /// macOS has not asked the user yet.
    NotAsked,
    /// macOS gives no way to ask; the user checks in System Settings.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRow {
    pub name: &'static str,
    pub why: &'static str,
    pub access: Access,
    pub pane: PrivacyPane,
}

impl PermissionRow {
    pub fn status_text(&self) -> &'static str {
        match self.access {
            Access::Granted => "Granted",
            Access::Denied => "Not granted",
            Access::NotAsked => "Not asked yet",
            Access::Unknown => "Check in System Settings",
        }
    }

    pub fn button_text(&self) -> &'static str {
        match self.access {
            Access::NotAsked => "Allow…",
            _ => "Open System Settings",
        }
    }
}

fn granted(yes: bool) -> Access {
    if yes {
        Access::Granted
    } else {
        Access::Denied
    }
}

/// Asks macOS for the current state. Has no side effects: nothing is requested.
pub fn rows(permissions: &Permissions) -> Vec<PermissionRow> {
    vec![
        PermissionRow {
            name: "Microphone",
            why: "to hear you",
            access: match (permissions.microphone_granted)() {
                Some(yes) => granted(yes),
                None => Access::NotAsked,
            },
            pane: PrivacyPane::Microphone,
        },
        PermissionRow {
            name: "Accessibility",
            why: "to catch your shortcuts, read the selected text and type into other apps",
            access: granted((permissions.accessibility_granted)()),
            pane: PrivacyPane::Accessibility,
        },
        PermissionRow {
            name: "Input Monitoring",
            why: "to notice your shortcuts",
            access: if (permissions.shortcuts_listening)() {
                Access::Granted
            } else {
                Access::Unknown
            },
            pane: PrivacyPane::InputMonitoring,
        },
    ]
}

/// The permission rows, re-read from macOS at most every `RECHECK_EVERY`.
pub struct PermissionSnapshot {
    rows: Vec<PermissionRow>,
    checked_at: Instant,
}

impl PermissionSnapshot {
    pub fn take(permissions: &Permissions, now: Instant) -> Self {
        Self {
            rows: rows(permissions),
            checked_at: now,
        }
    }

    /// The rows, refreshed first if the last check is `RECHECK_EVERY` old.
    pub fn current(&mut self, permissions: &Permissions, now: Instant) -> &[PermissionRow] {
        if now.duration_since(self.checked_at) >= RECHECK_EVERY {
            *self = Self::take(permissions, now);
        }
        &self.rows
    }
}

/// What the button of `row` does: a microphone never asked about gets macOS's own
/// request; missing accessibility gets macOS's prompt (it lists the app) and the
/// pane; everything else opens its pane.
pub fn open_settings(permissions: &Permissions, row: &PermissionRow) {
    match (row.pane, row.access) {
        (PrivacyPane::Microphone, Access::NotAsked) => (permissions.request_microphone)(),
        (PrivacyPane::Accessibility, access) if access != Access::Granted => {
            (permissions.prompt_accessibility)();
            (permissions.open_privacy_settings)(row.pane);
        }
        _ => (permissions.open_privacy_settings)(row.pane),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn permissions(
        mic: Option<bool>,
        ax: bool,
        listening: bool,
        log: Arc<Mutex<Vec<String>>>,
    ) -> Permissions {
        let prompt_log = log.clone();
        let request_log = log.clone();
        Permissions {
            microphone_granted: Arc::new(move || mic),
            request_microphone: Arc::new(move || {
                request_log.lock().unwrap().push("request mic".into())
            }),
            accessibility_granted: Arc::new(move || ax),
            prompt_accessibility: Arc::new(move || {
                prompt_log.lock().unwrap().push("prompt".into())
            }),
            shortcuts_listening: Arc::new(move || listening),
            open_privacy_settings: Arc::new(move |pane| {
                log.lock().unwrap().push(format!("open {pane:?}"))
            }),
        }
    }

    #[test]
    fn rows_reflect_what_macos_says() {
        let rows = rows(&permissions(Some(true), false, false, Arc::default()));
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name, "Microphone");
        assert_eq!(rows[0].access, Access::Granted);
        assert_eq!(rows[0].pane, PrivacyPane::Microphone);
        assert_eq!(rows[1].name, "Accessibility");
        assert_eq!(rows[1].access, Access::Denied);
        assert_eq!(rows[2].name, "Input Monitoring");
        assert_eq!(rows[2].access, Access::Unknown);
        assert_eq!(rows[2].pane, PrivacyPane::InputMonitoring);
    }

    #[test]
    fn undecided_microphone_and_listening_shortcuts() {
        let rows = rows(&permissions(None, true, true, Arc::default()));
        assert_eq!(rows[0].access, Access::NotAsked);
        assert_eq!(rows[2].access, Access::Granted);
    }

    #[test]
    fn reading_the_rows_requests_nothing() {
        let log = Arc::new(Mutex::new(Vec::new()));
        rows(&permissions(None, false, false, log.clone()));
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn status_and_button_texts() {
        let row = |access| PermissionRow {
            name: "x",
            why: "y",
            access,
            pane: PrivacyPane::Microphone,
        };
        assert_eq!(row(Access::Granted).status_text(), "Granted");
        assert_eq!(row(Access::Denied).status_text(), "Not granted");
        assert_eq!(row(Access::NotAsked).status_text(), "Not asked yet");
        assert_eq!(
            row(Access::Unknown).status_text(),
            "Check in System Settings"
        );
        assert_eq!(row(Access::NotAsked).button_text(), "Allow…");
        assert_eq!(row(Access::Denied).button_text(), "Open System Settings");
    }

    #[test]
    fn undecided_microphone_is_requested_not_opened() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let p = permissions(None, true, false, log.clone());
        open_settings(&p, &rows(&p)[0]);
        assert_eq!(*log.lock().unwrap(), ["request mic"]);
    }

    #[test]
    fn missing_accessibility_prompts_then_opens_the_pane() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let p = permissions(Some(true), false, false, log.clone());
        open_settings(&p, &rows(&p)[1]);
        assert_eq!(*log.lock().unwrap(), ["prompt", "open Accessibility"]);
    }

    #[test]
    fn other_rows_only_open_the_pane() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let p = permissions(Some(false), true, false, log.clone());
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

    #[test]
    fn snapshot_rechecks_only_after_the_interval() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let mut p = permissions(Some(true), true, false, Arc::default());
        p.accessibility_granted = Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            true
        });
        let start = Instant::now();
        let mut snap = PermissionSnapshot::take(&p, start);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        snap.current(&p, start + RECHECK_EVERY / 2);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let rows = snap.current(&p, start + RECHECK_EVERY).to_vec();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(rows.len(), 3);
    }
}
