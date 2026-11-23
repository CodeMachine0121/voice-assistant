use crate::{Action, PersonalDictionary};
use serde::{Deserialize, Serialize};

/// A global shortcut. Every shortcut starts its action when tapped and stops it when
/// tapped again. A "tap" shortcut counts only when its keys are pressed and released
/// with no other key or mouse button pressed in between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Shortcut {
    /// Tapping a Command key alone.
    TapCommand,
    /// Tapping a Command key together with a Shift key.
    TapCommandShift,
    /// Option + Space.
    OptionSpace,
    /// Tapping the right Option key alone.
    TapRightOption,
    /// Tapping the right Command key alone.
    TapRightCommand,
    /// Tapping the Fn key alone.
    TapFn,
    /// Control + Shift + Space.
    ControlShiftSpace,
}

impl Shortcut {
    /// Every shortcut the user may choose from, in the order they are offered.
    pub const ALL: [Shortcut; 7] = [
        Shortcut::TapCommand,
        Shortcut::TapCommandShift,
        Shortcut::OptionSpace,
        Shortcut::TapRightOption,
        Shortcut::TapRightCommand,
        Shortcut::TapFn,
        Shortcut::ControlShiftSpace,
    ];

    /// Whether one key press can trigger both shortcuts. Tapping the right Command key
    /// is also a tap of "a Command key", so those two collide like equal shortcuts.
    pub fn collides_with(self, other: Shortcut) -> bool {
        use Shortcut::{TapCommand, TapRightCommand};
        self == other
            || matches!(
                (self, other),
                (TapCommand, TapRightCommand) | (TapRightCommand, TapCommand)
            )
    }
}

/// The shortcut of each action. No two actions may share one (see `conflict`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActionShortcuts {
    pub dictate: Shortcut,
    pub english: Shortcut,
    pub edit_selection: Shortcut,
}

impl Default for ActionShortcuts {
    fn default() -> Self {
        Self {
            dictate: Shortcut::TapCommand,
            english: Shortcut::TapCommandShift,
            edit_selection: Shortcut::OptionSpace,
        }
    }
}

impl ActionShortcuts {
    pub fn for_action(&self, a: Action) -> &Shortcut {
        match a {
            Action::Dictate => &self.dictate,
            Action::DictateInEnglish => &self.english,
            Action::EditSelection => &self.edit_selection,
        }
    }

    pub fn set(&mut self, a: Action, s: Shortcut) {
        match a {
            Action::Dictate => self.dictate = s,
            Action::DictateInEnglish => self.english = s,
            Action::EditSelection => self.edit_selection = s,
        }
    }

    /// The first two actions whose shortcuts collide (see `Shortcut::collides_with`), as
    /// `(earlier, later)` in `Action::ALL` order; `None` when every action has its own.
    pub fn conflict(&self) -> Option<(Action, Action)> {
        Action::ALL.iter().enumerate().find_map(|(i, &earlier)| {
            Action::ALL[i + 1..]
                .iter()
                .find(|&&later| {
                    self.for_action(earlier)
                        .collides_with(*self.for_action(later))
                })
                .map(|&later| (earlier, later))
        })
    }
}

/// The online service that turns speech into words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SpeechProvider {
    #[default]
    OpenAi,
}

impl SpeechProvider {
    /// Every provider the user may choose from, in the order they are offered.
    pub const ALL: [SpeechProvider; 1] = [SpeechProvider::OpenAi];

    /// Name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            SpeechProvider::OpenAi => "OpenAI",
        }
    }

    /// Speech models offered for this provider; the first is the default.
    pub fn models(self) -> &'static [&'static str] {
        match self {
            SpeechProvider::OpenAi => &["gpt-4o-transcribe", "gpt-4o-mini-transcribe", "whisper-1"],
        }
    }

    pub fn default_model(self) -> &'static str {
        self.models()[0]
    }
}

/// Which provider and speech model turn recordings into words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpeechSettings {
    pub provider: SpeechProvider,
    /// The provider's identifier of the speech model.
    pub model: String,
}

impl Default for SpeechSettings {
    fn default() -> Self {
        let provider = SpeechProvider::default();
        Self {
            provider,
            model: provider.default_model().to_string(),
        }
    }
}

/// Settings saved by earlier versions load with every field they still share; fields that
/// no longer exist (the press-and-hold shortcuts, the on-device and local speech model
/// choices) are ignored and new ones take defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub shortcuts: ActionShortcuts,
    /// `None` = system default microphone.
    pub microphone: Option<String>,
    /// OpenAI-compatible base URL of the local AI app (LM Studio default).
    pub assistant_url: String,
    /// Writing assistant model as listed by the local AI app; empty = whatever is loaded.
    pub assistant_model: String,
    /// AI cleanup of Dictate results (has no effect on the other actions).
    pub cleanup_enabled: bool,
    pub dictionary: PersonalDictionary,
    pub speech: SpeechSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shortcuts: ActionShortcuts::default(),
            microphone: None,
            assistant_url: "http://localhost:1234/v1".to_string(),
            assistant_model: String::new(),
            cleanup_enabled: true,
            dictionary: PersonalDictionary::default(),
            speech: SpeechSettings::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_launch_defaults() {
        let s = Settings::default();
        assert_eq!(s.shortcuts.dictate, Shortcut::TapCommand);
        assert_eq!(s.shortcuts.english, Shortcut::TapCommandShift);
        assert_eq!(s.shortcuts.edit_selection, Shortcut::OptionSpace);
        assert_eq!(s.speech.provider, SpeechProvider::OpenAi);
        assert_eq!(s.speech.provider.label(), "OpenAI");
        assert_eq!(s.speech.model, "gpt-4o-transcribe");
        assert_eq!(s.assistant_model, "", "whatever is loaded");
        assert_eq!(s.assistant_url, "http://localhost:1234/v1");
        assert_eq!(s.microphone, None, "system default microphone");
        assert!(s.cleanup_enabled);
        assert!(s.dictionary.is_empty());
    }

    #[test]
    fn openai_is_the_only_speech_provider() {
        assert_eq!(SpeechProvider::ALL, [SpeechProvider::OpenAi]);
    }

    #[test]
    fn openai_offers_three_speech_models_with_gpt_4o_transcribe_first() {
        assert_eq!(
            SpeechProvider::OpenAi.models(),
            ["gpt-4o-transcribe", "gpt-4o-mini-transcribe", "whisper-1"]
        );
        assert_eq!(SpeechProvider::OpenAi.default_model(), "gpt-4o-transcribe");
    }

    #[test]
    fn first_launch_shortcuts_do_not_conflict() {
        assert_eq!(ActionShortcuts::default().conflict(), None);
    }

    #[test]
    fn each_action_reads_and_changes_its_own_shortcut() {
        let mut shortcuts = ActionShortcuts::default();

        shortcuts.set(Action::Dictate, Shortcut::TapRightOption);
        shortcuts.set(Action::DictateInEnglish, Shortcut::TapFn);
        shortcuts.set(Action::EditSelection, Shortcut::ControlShiftSpace);

        assert_eq!(
            shortcuts.for_action(Action::Dictate),
            &Shortcut::TapRightOption
        );
        assert_eq!(
            shortcuts.for_action(Action::DictateInEnglish),
            &Shortcut::TapFn
        );
        assert_eq!(
            shortcuts.for_action(Action::EditSelection),
            &Shortcut::ControlShiftSpace
        );
        assert_eq!(shortcuts.conflict(), None);
    }

    #[test]
    fn changing_dictate_to_tap_right_option_frees_tap_command() {
        let mut shortcuts = ActionShortcuts::default();

        shortcuts.set(Action::Dictate, Shortcut::TapRightOption);

        assert!(Action::ALL
            .iter()
            .all(|&a| shortcuts.for_action(a) != &Shortcut::TapCommand));
        assert_eq!(shortcuts.conflict(), None);
    }

    #[test]
    fn two_actions_sharing_a_shortcut_conflict_naming_the_earlier_first() {
        let mut shortcuts = ActionShortcuts::default();

        shortcuts.set(Action::EditSelection, Shortcut::TapCommand);

        assert_eq!(
            shortcuts.conflict(),
            Some((Action::Dictate, Action::EditSelection))
        );
    }

    #[test]
    fn tapping_any_command_collides_with_tapping_the_right_command() {
        let mut shortcuts = ActionShortcuts::default();
        shortcuts.set(Action::EditSelection, Shortcut::TapRightCommand);

        assert_eq!(
            shortcuts.conflict(),
            Some((Action::Dictate, Action::EditSelection))
        );
    }

    #[test]
    fn distinct_shortcuts_do_not_collide() {
        for a in Shortcut::ALL {
            for b in Shortcut::ALL {
                let overlapping_command = matches!(
                    (a, b),
                    (Shortcut::TapCommand, Shortcut::TapRightCommand)
                        | (Shortcut::TapRightCommand, Shortcut::TapCommand)
                );
                assert_eq!(
                    a.collides_with(b),
                    a == b || overlapping_command,
                    "{a:?} {b:?}"
                );
            }
        }
    }

    #[test]
    fn a_conflict_between_the_later_actions_is_found() {
        let mut shortcuts = ActionShortcuts::default();

        shortcuts.set(Action::EditSelection, Shortcut::TapCommandShift);

        assert_eq!(
            shortcuts.conflict(),
            Some((Action::DictateInEnglish, Action::EditSelection))
        );
    }

    #[test]
    fn every_shortcut_is_offered_once() {
        let mut seen = std::collections::HashSet::new();
        assert!(Shortcut::ALL.iter().all(|s| seen.insert(*s)));
        assert_eq!(seen.len(), 7);
    }

    #[test]
    fn settings_round_trip_through_json() {
        let mut settings = Settings {
            speech: SpeechSettings {
                provider: SpeechProvider::OpenAi,
                model: "gpt-4o-mini-transcribe".into(),
            },
            assistant_model: "twil-8b".into(),
            ..Settings::default()
        };
        settings.shortcuts.set(Action::Dictate, Shortcut::TapFn);

        let json = serde_json::to_string(&settings).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();

        assert_eq!(back, settings);
    }

    #[test]
    fn settings_from_the_press_and_hold_version_keep_everything_but_the_shortcuts() {
        let old = r#"{
            "push_to_talk": "RightOption",
            "hands_free": "ControlOptionSpace",
            "microphone": "USB Mic",
            "assistant_url": "http://localhost:4321/v1",
            "assistant_model": "twil-8b",
            "cleanup_enabled": false,
            "dictionary": ["Cafler"],
            "speech_model_path": "/models/ggml.bin"
        }"#;

        let s: Settings = serde_json::from_str(old).unwrap();

        assert_eq!(s.shortcuts, ActionShortcuts::default());
        assert_eq!(s.speech, SpeechSettings::default());
        assert_eq!(s.dictionary.words(), ["Cafler"]);
        assert_eq!(s.microphone.as_deref(), Some("USB Mic"));
        assert_eq!(s.assistant_url, "http://localhost:4321/v1");
        assert_eq!(s.assistant_model, "twil-8b");
        assert!(!s.cleanup_enabled);
    }

    #[test]
    fn a_saved_built_in_speech_choice_moves_to_openai_and_keeps_the_rest() {
        let old = r#"{
            "shortcuts": { "dictate": "TapFn" },
            "microphone": "USB Mic",
            "assistant_model": "twil-8b",
            "dictionary": ["Cafler"],
            "speech_model_path": "/models/ggml-small.bin",
            "speech_source": "BuiltIn"
        }"#;

        let s: Settings = serde_json::from_str(old).unwrap();

        assert_eq!(s.speech.provider, SpeechProvider::OpenAi);
        assert_eq!(s.speech.model, "gpt-4o-transcribe");
        assert_eq!(s.shortcuts.dictate, Shortcut::TapFn);
        assert_eq!(s.microphone.as_deref(), Some("USB Mic"));
        assert_eq!(s.dictionary.words(), ["Cafler"]);
        assert_eq!(s.assistant_model, "twil-8b");
    }

    #[test]
    fn a_saved_local_ai_app_speech_model_moves_to_openai() {
        let old = r#"{
            "assistant_model": "twil-8b",
            "speech_source": { "LocalAiApp": { "model": "breeze-asr-25" } }
        }"#;

        let s: Settings = serde_json::from_str(old).unwrap();

        assert_eq!(s.speech.provider, SpeechProvider::OpenAi);
        assert_eq!(s.speech.model, "gpt-4o-transcribe");
        assert_eq!(s.assistant_model, "twil-8b");
    }

    #[test]
    fn settings_without_a_speech_choice_use_openai() {
        let s: Settings = serde_json::from_str("{}").unwrap();

        assert_eq!(s.speech.provider, SpeechProvider::OpenAi);
        assert_eq!(s.speech.model, "gpt-4o-transcribe");
    }

    #[test]
    fn a_partially_saved_speech_choice_fills_in_the_provider() {
        let s: Settings =
            serde_json::from_str(r#"{ "speech": { "model": "whisper-1" } }"#).unwrap();

        assert_eq!(s.speech.provider, SpeechProvider::OpenAi);
        assert_eq!(s.speech.model, "whisper-1");
    }

    #[test]
    fn partially_saved_shortcuts_fill_in_the_missing_ones() {
        let s: Settings =
            serde_json::from_str(r#"{ "shortcuts": { "dictate": "TapFn" } }"#).unwrap();

        assert_eq!(s.shortcuts.dictate, Shortcut::TapFn);
        assert_eq!(s.shortcuts.english, Shortcut::TapCommandShift);
        assert_eq!(s.shortcuts.edit_selection, Shortcut::OptionSpace);
    }
}
