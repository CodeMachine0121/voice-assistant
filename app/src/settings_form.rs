//! Editable copy of `Settings` for the Settings window, and the conversion back.
use std::path::PathBuf;
use voice_core::{PersonalDictionary, Settings, Shortcut};

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsForm {
    pub push_to_talk: Shortcut,
    pub hands_free: Shortcut,
    /// `None` = system default microphone.
    pub microphone: Option<String>,
    pub assistant_url: String,
    pub assistant_model: String,
    pub cleanup_enabled: bool,
    dictionary: PersonalDictionary,
    /// Text typed into the "add word" field.
    pub new_word: String,
    /// Speech model location as typed; blank = default location.
    pub speech_model_path: String,
}

impl SettingsForm {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            push_to_talk: settings.push_to_talk.clone(),
            hands_free: settings.hands_free.clone(),
            microphone: settings.microphone.clone(),
            assistant_url: settings.assistant_url.clone(),
            assistant_model: settings.assistant_model.clone(),
            cleanup_enabled: settings.cleanup_enabled,
            dictionary: settings.dictionary.clone(),
            new_word: String::new(),
            speech_model_path: settings
                .speech_model_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        }
    }

    pub fn to_settings(&self) -> Settings {
        let path = self.speech_model_path.trim();
        Settings {
            push_to_talk: self.push_to_talk.clone(),
            hands_free: self.hands_free.clone(),
            microphone: self.microphone.clone(),
            assistant_url: self.assistant_url.trim().to_string(),
            assistant_model: self.assistant_model.trim().to_string(),
            cleanup_enabled: self.cleanup_enabled,
            dictionary: self.dictionary.clone(),
            speech_model_path: (!path.is_empty()).then(|| PathBuf::from(path)),
        }
    }

    /// Adds the typed word to the dictionary and empties the field.
    /// Returns whether the dictionary changed (blanks and duplicates are ignored).
    pub fn add_word(&mut self) -> bool {
        let word = std::mem::take(&mut self.new_word);
        self.dictionary.add(&word)
    }

    pub fn remove_word(&mut self, word: &str) {
        self.dictionary.remove(word);
    }

    pub fn words(&self) -> &[String] {
        self.dictionary.words()
    }

    /// Why the form cannot be saved yet, if it cannot.
    pub fn problem(&self) -> Option<&'static str> {
        if self.push_to_talk == self.hands_free {
            Some("Hold to talk and Hands-free need different shortcuts.")
        } else if self.cleanup_enabled && self.assistant_url.trim().is_empty() {
            Some("Enter the writing assistant address, or turn AI cleanup off.")
        } else {
            None
        }
    }

    /// Whether the form differs from what is stored (including a half-typed word).
    pub fn is_changed_from(&self, stored: &Settings) -> bool {
        self.to_settings() != *stored || !self.new_word.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> SettingsForm {
        SettingsForm::from_settings(&Settings::default())
    }

    #[test]
    fn untouched_form_round_trips_defaults() {
        assert_eq!(form().to_settings(), Settings::default());
    }

    #[test]
    fn untouched_form_round_trips_a_full_settings_value() {
        let settings = Settings {
            push_to_talk: Shortcut::Fn,
            hands_free: Shortcut::ControlShiftSpace,
            microphone: Some("USB Mic".into()),
            assistant_url: "http://host:1/v1".into(),
            assistant_model: "qwen".into(),
            cleanup_enabled: false,
            dictionary: PersonalDictionary::from_words(["Kubernetes", "Zoë"]),
            speech_model_path: Some(PathBuf::from("/models/small.bin")),
        };
        assert_eq!(
            SettingsForm::from_settings(&settings).to_settings(),
            settings
        );
    }

    #[test]
    fn edits_show_up_in_settings() {
        let mut f = form();
        f.push_to_talk = Shortcut::Fn;
        f.hands_free = Shortcut::ControlShiftSpace;
        f.microphone = Some("Headset".into());
        f.assistant_url = "http://other/v1".into();
        f.assistant_model = "llama".into();
        f.cleanup_enabled = false;
        let s = f.to_settings();
        assert_eq!(s.push_to_talk, Shortcut::Fn);
        assert_eq!(s.hands_free, Shortcut::ControlShiftSpace);
        assert_eq!(s.microphone.as_deref(), Some("Headset"));
        assert_eq!(s.assistant_url, "http://other/v1");
        assert_eq!(s.assistant_model, "llama");
        assert!(!s.cleanup_enabled);
    }

    #[test]
    fn text_fields_are_trimmed() {
        let mut f = form();
        f.assistant_url = "  http://x/v1 \n".into();
        f.assistant_model = " m ".into();
        let s = f.to_settings();
        assert_eq!(s.assistant_url, "http://x/v1");
        assert_eq!(s.assistant_model, "m");
    }

    #[test]
    fn blank_model_path_means_default_location() {
        let mut f = form();
        f.speech_model_path = "   ".into();
        assert_eq!(f.to_settings().speech_model_path, None);
    }

    #[test]
    fn typed_model_path_is_kept() {
        let mut f = form();
        f.speech_model_path = " /a/b.bin ".into();
        assert_eq!(
            f.to_settings().speech_model_path,
            Some(PathBuf::from("/a/b.bin"))
        );
    }

    #[test]
    fn existing_model_path_is_shown() {
        let s = Settings {
            speech_model_path: Some(PathBuf::from("/a/b.bin")),
            ..Settings::default()
        };
        assert_eq!(
            SettingsForm::from_settings(&s).speech_model_path,
            "/a/b.bin"
        );
    }

    #[test]
    fn adding_a_word_updates_dictionary_and_clears_the_field() {
        let mut f = form();
        f.new_word = "  Kubernetes ".into();
        assert!(f.add_word());
        assert_eq!(f.new_word, "");
        assert_eq!(f.words(), ["Kubernetes"]);
        assert_eq!(f.to_settings().dictionary.words(), ["Kubernetes"]);
    }

    #[test]
    fn blank_word_is_ignored() {
        let mut f = form();
        f.new_word = "   ".into();
        assert!(!f.add_word());
        assert!(f.words().is_empty());
        assert_eq!(f.new_word, "");
    }

    #[test]
    fn duplicate_word_is_ignored() {
        let mut f = form();
        f.new_word = "Rust".into();
        f.add_word();
        f.new_word = "Rust".into();
        assert!(!f.add_word());
        assert_eq!(f.words(), ["Rust"]);
    }

    #[test]
    fn words_keep_insertion_order_and_can_be_removed() {
        let mut f = form();
        for w in ["b", "a", "c"] {
            f.new_word = w.into();
            f.add_word();
        }
        assert_eq!(f.words(), ["b", "a", "c"]);
        f.remove_word("a");
        assert_eq!(f.words(), ["b", "c"]);
        f.remove_word("missing");
        assert_eq!(f.words(), ["b", "c"]);
    }

    #[test]
    fn same_shortcut_twice_blocks_saving() {
        let mut f = form();
        f.hands_free = f.push_to_talk.clone();
        assert!(f.problem().unwrap().contains("different shortcuts"));
    }

    #[test]
    fn blank_url_blocks_saving_only_when_cleanup_is_on() {
        let mut f = form();
        f.assistant_url = "  ".into();
        assert!(f.problem().unwrap().contains("address"));
        f.cleanup_enabled = false;
        assert_eq!(f.problem(), None);
    }

    #[test]
    fn defaults_are_savable() {
        assert_eq!(form().problem(), None);
    }

    #[test]
    fn change_detection() {
        let stored = Settings::default();
        let mut f = SettingsForm::from_settings(&stored);
        assert!(!f.is_changed_from(&stored));
        f.cleanup_enabled = false;
        assert!(f.is_changed_from(&stored));
        f.cleanup_enabled = true;
        f.new_word = "pending".into();
        assert!(f.is_changed_from(&stored));
        f.new_word = "  ".into();
        assert!(!f.is_changed_from(&stored));
    }
}
