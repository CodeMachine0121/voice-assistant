//! Disk adapter: settings and history as JSON files.
mod file;

use file::JsonFile;
use serde::{de::DeserializeOwned, Serialize};
use std::ffi::OsString;
use std::path::PathBuf;
use voice_core::{History, HistoryStore, Settings, SettingsStore};

const SETTINGS_FILE: &str = "settings.json";
const HISTORY_FILE: &str = "history.json";

/// `<home>/Library/Application Support/VoiceAssistant/`; a relative folder without a home.
fn dir_under(home: Option<OsString>) -> PathBuf {
    match home.filter(|h| !h.is_empty()) {
        Some(home) => PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("VoiceAssistant"),
        None => PathBuf::from("VoiceAssistant"),
    }
}

fn in_default_dir() -> PathBuf {
    dir_under(std::env::var_os("HOME"))
}

fn load_or_default<T: DeserializeOwned + Default>(file: &JsonFile) -> T {
    file.load().unwrap_or_default()
}

fn save<T: Serialize>(file: &JsonFile, value: &T) -> Result<(), String> {
    file.save(value)
}

/// Settings kept in `settings.json`.
#[derive(Debug, Clone)]
pub struct JsonSettingsStore {
    file: JsonFile,
}

impl JsonSettingsStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            file: JsonFile::new(dir.join(SETTINGS_FILE)),
        }
    }

    pub fn in_default_location() -> Self {
        Self::new(in_default_dir())
    }
}

impl SettingsStore for JsonSettingsStore {
    fn load(&self) -> Settings {
        load_or_default(&self.file)
    }

    fn save(&self, settings: &Settings) -> Result<(), String> {
        save(&self.file, settings)
    }
}

/// History kept in `history.json`.
#[derive(Debug, Clone)]
pub struct JsonHistoryStore {
    file: JsonFile,
}

impl JsonHistoryStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            file: JsonFile::new(dir.join(HISTORY_FILE)),
        }
    }

    pub fn in_default_location() -> Self {
        Self::new(in_default_dir())
    }
}

impl HistoryStore for JsonHistoryStore {
    fn load(&self) -> History {
        load_or_default(&self.file)
    }

    fn save(&self, history: &History) -> Result<(), String> {
        save(&self.file, history)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};
    use voice_core::{HistoryEntry, PersonalDictionary, Shortcut};

    fn entry(n: u32) -> HistoryEntry {
        HistoryEntry {
            at: Local.with_ymd_and_hms(2026, 11, 22, 9, 0, n).unwrap(),
            raw: format!("um raw {n}"),
            polished: format!("Polished {n}."),
        }
    }

    #[test]
    fn first_launch_has_default_settings() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            JsonSettingsStore::new(dir.path().into()).load(),
            Settings::default()
        );
    }

    #[test]
    fn settings_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonSettingsStore::new(dir.path().into());
        let s = Settings {
            assistant_model: "qwen2.5-7b-instruct".into(),
            push_to_talk: Shortcut::Fn,
            microphone: Some("USB Mic".into()),
            cleanup_enabled: false,
            dictionary: PersonalDictionary::from_words(["Cafler"]),
            speech_model_path: Some("/models/ggml.bin".into()),
            ..Settings::default()
        };
        store.save(&s).unwrap();
        assert_eq!(store.load(), s);
    }

    #[test]
    fn settings_are_remembered_across_instances() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings {
            assistant_model: "qwen2.5-7b-instruct".into(),
            ..Settings::default()
        };
        JsonSettingsStore::new(dir.path().into()).save(&s).unwrap();
        let restarted = JsonSettingsStore::new(dir.path().into());
        assert_eq!(restarted.load().assistant_model, "qwen2.5-7b-instruct");
    }

    #[test]
    fn save_creates_missing_directories_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        JsonSettingsStore::new(nested.clone())
            .save(&Settings::default())
            .unwrap();
        let names: Vec<_> = std::fs::read_dir(&nested)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["settings.json"]);
    }

    #[test]
    fn saved_json_is_pretty() {
        let dir = tempfile::tempdir().unwrap();
        JsonSettingsStore::new(dir.path().into())
            .save(&Settings::default())
            .unwrap();
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(text.contains("\n  \"assistant_url\""));
    }

    #[test]
    fn save_overwrites_previous_content() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonSettingsStore::new(dir.path().into());
        let with_model = |m: &str| Settings {
            assistant_model: m.into(),
            ..Settings::default()
        };
        store.save(&with_model("a")).unwrap();
        store.save(&with_model("b")).unwrap();
        assert_eq!(store.load().assistant_model, "b");
    }

    #[test]
    fn corrupt_settings_give_defaults_and_keep_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), "{ not json").unwrap();
        let store = JsonSettingsStore::new(dir.path().into());
        assert_eq!(store.load(), Settings::default());
        let bak = std::fs::read_to_string(dir.path().join("settings.json.bak")).unwrap();
        assert_eq!(bak, "{ not json");
    }

    #[test]
    fn corrupt_history_gives_empty_and_keeps_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("history.json"), "[1,2").unwrap();
        let store = JsonHistoryStore::new(dir.path().into());
        assert_eq!(store.load(), History::default());
        assert!(dir.path().join("history.json.bak").exists());
    }

    #[test]
    fn missing_history_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(JsonHistoryStore::new(dir.path().into())
            .load()
            .entries()
            .is_empty());
    }

    #[test]
    fn history_keeps_newest_first_across_instances() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = History::default();
        for n in 1..=3 {
            h.record(entry(n));
        }
        JsonHistoryStore::new(dir.path().into()).save(&h).unwrap();
        let loaded = JsonHistoryStore::new(dir.path().into()).load();
        assert_eq!(loaded, h);
        let polished: Vec<_> = loaded
            .entries()
            .iter()
            .map(|e| e.polished.as_str())
            .collect();
        assert_eq!(polished, ["Polished 3.", "Polished 2.", "Polished 1."]);
    }

    #[test]
    fn save_fails_with_a_message_when_the_directory_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        let err = JsonSettingsStore::new(blocker)
            .save(&Settings::default())
            .unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn default_location_is_under_application_support() {
        assert_eq!(
            dir_under(Some("/Users/me".into())),
            PathBuf::from("/Users/me/Library/Application Support/VoiceAssistant")
        );
        // Constructors resolve without touching the disk.
        let _ = JsonSettingsStore::in_default_location();
        let _ = JsonHistoryStore::in_default_location();
    }

    #[test]
    fn without_a_home_directory_a_relative_folder_is_used() {
        assert_eq!(dir_under(None), PathBuf::from("VoiceAssistant"));
        assert_eq!(dir_under(Some("".into())), PathBuf::from("VoiceAssistant"));
    }

    #[test]
    fn unreadable_settings_give_defaults() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be cannot be read as text.
        std::fs::create_dir(dir.path().join("settings.json")).unwrap();
        assert_eq!(
            JsonSettingsStore::new(dir.path().into()).load(),
            Settings::default()
        );
    }

    #[test]
    fn corrupt_file_is_still_ignored_when_the_backup_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), "oops").unwrap();
        std::fs::create_dir(dir.path().join("settings.json.bak")).unwrap();
        assert_eq!(
            JsonSettingsStore::new(dir.path().into()).load(),
            Settings::default()
        );
    }

    #[test]
    fn save_fails_when_the_target_cannot_be_replaced() {
        let dir = tempfile::tempdir().unwrap();
        // A non-empty directory at the target path blocks the final rename.
        let target = dir.path().join("settings.json");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("keep"), "x").unwrap();
        let err = JsonSettingsStore::new(dir.path().into())
            .save(&Settings::default())
            .unwrap_err();
        assert!(err.contains("cannot replace"), "{err}");
    }

    #[test]
    fn save_fails_when_the_temp_file_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("settings.json.tmp")).unwrap();
        let err = JsonSettingsStore::new(dir.path().into())
            .save(&Settings::default())
            .unwrap_err();
        assert!(err.contains("cannot write"), "{err}");
    }
}
