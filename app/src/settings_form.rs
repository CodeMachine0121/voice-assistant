//! Editable copy of `Settings` for the Settings window, and the conversion back.
use std::path::PathBuf;
use voice_core::{
    Action, ActionShortcuts, PersonalDictionary, Settings, SpeechSettings, SpeechSource,
};

/// The speech model row's first choice.
pub const BUILT_IN_SPEECH: &str = "Built-in (on this Mac)";
/// The writing assistant row's first choice.
pub const ANY_ASSISTANT_MODEL: &str = "Whatever is loaded";
/// Shown next to the address when it is empty.
pub const ADDRESS_MISSING: &str = "Enter the local AI app address.";
/// Shown when the address points away from this Mac.
pub const ADDRESS_NOT_LOCAL: &str =
    "This address is not on this Mac — what you say and select will leave the computer.";

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsForm {
    pub shortcuts: ActionShortcuts,
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
    pub speech_source: SpeechSource,
    pub speech: SpeechSettings,
}

impl SettingsForm {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            shortcuts: settings.shortcuts.clone(),
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
            speech_source: settings.speech_source.clone(),
            speech: settings.speech.clone(),
        }
    }

    pub fn to_settings(&self) -> Settings {
        let path = self.speech_model_path.trim();
        Settings {
            shortcuts: self.shortcuts.clone(),
            microphone: self.microphone.clone(),
            assistant_url: self.assistant_url.trim().to_string(),
            assistant_model: self.assistant_model.trim().to_string(),
            cleanup_enabled: self.cleanup_enabled,
            dictionary: self.dictionary.clone(),
            speech_model_path: (!path.is_empty()).then(|| PathBuf::from(path)),
            speech_source: self.speech_source.clone(),
            speech: self.speech.clone(),
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

    /// "Already used by <action>" for the row of `action`, when an action above it in
    /// the list has a colliding shortcut (see `Shortcut::collides_with`).
    pub fn shortcut_conflict(&self, action: Action) -> Option<String> {
        let mine = self.shortcuts.for_action(action);
        Action::ALL
            .iter()
            .take_while(|&&a| a != action)
            .find(|&&a| self.shortcuts.for_action(a).collides_with(*mine))
            .map(|a| format!("Already used by {}", a.label()))
    }

    /// Why the form cannot be saved yet, if it cannot.
    pub fn problem(&self) -> Option<String> {
        if let Some((earlier, later)) = self.shortcuts.conflict() {
            Some(format!(
                "{} and {} need different shortcuts.",
                earlier.label(),
                later.label()
            ))
        } else {
            self.address_problem().map(str::to_string)
        }
    }

    /// The part of `problem` that is not shown under a shortcut row.
    pub fn address_problem(&self) -> Option<&'static str> {
        self.assistant_url
            .trim()
            .is_empty()
            .then_some(ADDRESS_MISSING)
    }

    /// A warning when the local AI app address is not on this Mac: speech and selected
    /// text are sent there.
    pub fn address_warning(&self) -> Option<&'static str> {
        let address = self.assistant_url.trim();
        (!address.is_empty() && !is_on_this_mac(address)).then_some(ADDRESS_NOT_LOCAL)
    }

    /// Whether the built-in speech model's location and download are relevant.
    pub fn shows_built_in_speech_controls(&self) -> bool {
        self.speech_source == SpeechSource::BuiltIn
    }

    /// Whether the form differs from what is stored (including a half-typed word).
    pub fn is_changed_from(&self, stored: &Settings) -> bool {
        self.to_settings() != *stored || !self.new_word.trim().is_empty()
    }
}

/// Whether `address` (a URL, with or without scheme) names this Mac.
fn is_on_this_mac(address: &str) -> bool {
    let rest = address.split_once("://").map_or(address, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = match authority.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => host.eq_ignore_ascii_case("localhost"),
    }
}

/// Row label of a speech source.
pub fn speech_label(source: &SpeechSource) -> String {
    match source {
        SpeechSource::BuiltIn => BUILT_IN_SPEECH.to_string(),
        SpeechSource::LocalAiApp { model } => model.clone(),
    }
}

/// Row label of a writing assistant model (`""` = whatever is loaded).
pub fn assistant_label(model: &str) -> String {
    if model.is_empty() {
        ANY_ASSISTANT_MODEL.to_string()
    } else {
        model.to_string()
    }
}

/// Speech model choices: built-in first, then every listed model. A chosen model the
/// local AI app no longer lists stays on offer.
pub fn speech_choices(listed: &[String], current: &SpeechSource) -> Vec<SpeechSource> {
    let mut choices = vec![SpeechSource::BuiltIn];
    choices.extend(
        listed
            .iter()
            .map(|m| SpeechSource::LocalAiApp { model: m.clone() }),
    );
    if !choices.contains(current) {
        choices.push(current.clone());
    }
    choices
}

/// Writing assistant model choices: "whatever is loaded" (empty) first, then every
/// listed model. A chosen model the local AI app no longer lists stays on offer.
pub fn assistant_choices(listed: &[String], current: &str) -> Vec<String> {
    let mut choices = vec![String::new()];
    choices.extend(listed.iter().cloned());
    if !choices.iter().any(|c| c == current) {
        choices.push(current.to_string());
    }
    choices
}

#[cfg(test)]
mod tests {
    use super::*;
    use voice_core::Shortcut;

    fn form() -> SettingsForm {
        SettingsForm::from_settings(&Settings::default())
    }

    fn models(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn untouched_form_round_trips_defaults() {
        assert_eq!(form().to_settings(), Settings::default());
    }

    #[test]
    fn untouched_form_round_trips_a_full_settings_value() {
        let settings = Settings {
            shortcuts: ActionShortcuts {
                dictate: Shortcut::TapFn,
                english: Shortcut::ControlShiftSpace,
                edit_selection: Shortcut::TapRightOption,
            },
            microphone: Some("USB Mic".into()),
            assistant_url: "http://host:1/v1".into(),
            assistant_model: "qwen".into(),
            cleanup_enabled: false,
            dictionary: PersonalDictionary::from_words(["Kubernetes", "Zoë"]),
            speech_model_path: Some(PathBuf::from("/models/small.bin")),
            speech_source: SpeechSource::LocalAiApp {
                model: "breeze-asr-25".into(),
            },
            speech: SpeechSettings {
                model: "whisper-1".into(),
                ..SpeechSettings::default()
            },
        };
        assert_eq!(
            SettingsForm::from_settings(&settings).to_settings(),
            settings
        );
    }

    #[test]
    fn edits_show_up_in_settings() {
        let mut f = form();
        f.shortcuts.set(Action::Dictate, Shortcut::TapRightOption);
        f.microphone = Some("Headset".into());
        f.assistant_url = "http://other/v1".into();
        f.assistant_model = "llama".into();
        f.cleanup_enabled = false;
        f.speech_source = SpeechSource::LocalAiApp {
            model: "breeze-asr-25".into(),
        };
        let s = f.to_settings();
        assert_eq!(s.shortcuts.dictate, Shortcut::TapRightOption);
        assert_eq!(s.shortcuts.english, Shortcut::TapCommandShift);
        assert_eq!(s.microphone.as_deref(), Some("Headset"));
        assert_eq!(s.assistant_url, "http://other/v1");
        assert_eq!(s.assistant_model, "llama");
        assert!(!s.cleanup_enabled);
        assert_eq!(
            s.speech_source,
            SpeechSource::LocalAiApp {
                model: "breeze-asr-25".into()
            }
        );
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
    fn first_launch_rows_have_no_conflict() {
        let f = form();
        for action in Action::ALL {
            assert_eq!(f.shortcut_conflict(action), None);
        }
        assert_eq!(f.problem(), None);
    }

    #[test]
    fn a_shared_shortcut_is_reported_under_the_later_row_and_blocks_saving() {
        let mut f = form();
        f.shortcuts.set(Action::EditSelection, Shortcut::TapCommand);
        assert_eq!(f.shortcut_conflict(Action::Dictate), None);
        assert_eq!(f.shortcut_conflict(Action::DictateInEnglish), None);
        assert_eq!(
            f.shortcut_conflict(Action::EditSelection).as_deref(),
            Some("Already used by Dictate")
        );
        assert!(f.problem().is_some());
    }

    #[test]
    fn a_right_command_tap_is_reported_as_used_by_the_command_tap() {
        let mut f = form();
        f.shortcuts
            .set(Action::EditSelection, Shortcut::TapRightCommand);
        assert_eq!(
            f.shortcut_conflict(Action::EditSelection).as_deref(),
            Some("Already used by Dictate")
        );
        assert!(f.problem().is_some(), "saving stays blocked");
    }

    #[test]
    fn addresses_on_this_mac_get_no_warning() {
        let mut f = form();
        for address in [
            "http://localhost:1234/v1",
            "http://127.0.0.1:1234/v1",
            "http://127.1.2.3/v1",
            "http://[::1]:1234/v1",
            "localhost:1234",
            "HTTP://LOCALHOST/v1",
            "",
        ] {
            f.assistant_url = address.to_string();
            assert_eq!(f.address_warning(), None, "{address}");
        }
    }

    #[test]
    fn addresses_elsewhere_are_warned_about() {
        let mut f = form();
        for address in [
            "http://192.168.1.20:1234/v1",
            "https://api.example.com/v1",
            "http://localhost.example.com/v1",
            "http://localhost@evil.example/v1",
            "http://127.0.0.1.nip.io:1234/v1",
            "http://127.example.com/v1",
        ] {
            f.assistant_url = address.to_string();
            assert_eq!(f.address_warning(), Some(ADDRESS_NOT_LOCAL), "{address}");
        }
    }

    #[test]
    fn the_conflict_names_whichever_action_comes_first() {
        let mut f = form();
        f.shortcuts
            .set(Action::EditSelection, Shortcut::TapCommandShift);
        assert_eq!(
            f.shortcut_conflict(Action::EditSelection).as_deref(),
            Some("Already used by English")
        );
    }

    #[test]
    fn three_actions_on_one_shortcut_flag_both_later_rows() {
        let mut f = form();
        f.shortcuts
            .set(Action::DictateInEnglish, Shortcut::TapCommand);
        f.shortcuts.set(Action::EditSelection, Shortcut::TapCommand);
        assert_eq!(f.shortcut_conflict(Action::Dictate), None);
        assert_eq!(
            f.shortcut_conflict(Action::DictateInEnglish).as_deref(),
            Some("Already used by Dictate")
        );
        assert_eq!(
            f.shortcut_conflict(Action::EditSelection).as_deref(),
            Some("Already used by Dictate")
        );
    }

    #[test]
    fn changing_dictate_to_another_shortcut_is_fine() {
        let mut f = form();
        f.shortcuts.set(Action::Dictate, Shortcut::TapRightOption);
        assert_eq!(f.problem(), None);
        assert_eq!(f.to_settings().shortcuts.dictate, Shortcut::TapRightOption);
    }

    #[test]
    fn blank_address_blocks_saving() {
        let mut f = form();
        f.assistant_url = "  ".into();
        assert_eq!(
            f.problem().as_deref(),
            Some("Enter the local AI app address.")
        );
        f.cleanup_enabled = false;
        assert!(f.problem().is_some(), "English and Edit still need it");
    }

    #[test]
    fn defaults_are_savable() {
        assert_eq!(form().problem(), None);
    }

    #[test]
    fn first_launch_model_choices_are_built_in_and_whatever_is_loaded() {
        let f = form();
        assert_eq!(speech_label(&f.speech_source), "Built-in (on this Mac)");
        assert_eq!(assistant_label(&f.assistant_model), "Whatever is loaded");
    }

    #[test]
    fn model_choices_offer_every_listed_model() {
        let listed = models(&["breeze-asr-25", "twil-8b"]);
        let speech: Vec<_> = speech_choices(&listed, &SpeechSource::BuiltIn)
            .iter()
            .map(speech_label)
            .collect();
        assert_eq!(
            speech,
            ["Built-in (on this Mac)", "breeze-asr-25", "twil-8b"]
        );
        let assistant: Vec<_> = assistant_choices(&listed, "")
            .iter()
            .map(|m| assistant_label(m))
            .collect();
        assert_eq!(
            assistant,
            ["Whatever is loaded", "breeze-asr-25", "twil-8b"]
        );
    }

    #[test]
    fn without_a_listing_only_the_defaults_are_offered() {
        assert_eq!(
            speech_choices(&[], &SpeechSource::BuiltIn),
            [SpeechSource::BuiltIn]
        );
        assert_eq!(assistant_choices(&[], ""), [String::new()]);
    }

    #[test]
    fn a_chosen_model_that_is_no_longer_listed_stays_on_offer() {
        let chosen = SpeechSource::LocalAiApp {
            model: "gone".into(),
        };
        assert_eq!(
            speech_choices(&models(&["a"]), &chosen),
            [
                SpeechSource::BuiltIn,
                SpeechSource::LocalAiApp { model: "a".into() },
                chosen,
            ]
        );
        assert_eq!(
            assistant_choices(&models(&["a"]), "gone"),
            ["", "a", "gone"]
        );
    }

    #[test]
    fn built_in_controls_show_only_for_the_built_in_model() {
        let mut f = form();
        assert!(f.shows_built_in_speech_controls());
        f.speech_source = SpeechSource::LocalAiApp { model: "m".into() };
        assert!(!f.shows_built_in_speech_controls());
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
        f.speech_source = SpeechSource::LocalAiApp { model: "m".into() };
        assert!(f.is_changed_from(&stored));
    }
}
