use crate::{Action, PersonalDictionary};
use serde::{Deserialize, Serialize};

/// A global shortcut. There are exactly three, one per action, and they are fixed.
/// Every modifier in them is the right-hand key only: the left Command, Shift and
/// Option keys never trigger anything. Every shortcut starts its action when used
/// and stops it when used again. A "tap" counts only when its keys are pressed and
/// released with no other key or mouse button pressed in between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shortcut {
    /// Tapping the right Command key alone.
    TapRightCommand,
    /// Tapping the right Command key together with the right Shift key.
    TapRightCommandRightShift,
    /// The right Option key + Space.
    RightOptionSpace,
}

impl Shortcut {
    /// Every shortcut, in the order of the actions they trigger.
    pub const ALL: [Shortcut; 3] = [
        Shortcut::TapRightCommand,
        Shortcut::TapRightCommandRightShift,
        Shortcut::RightOptionSpace,
    ];

    /// The shortcut that triggers `action`.
    pub fn for_action(action: Action) -> Shortcut {
        match action {
            Action::Dictate => Shortcut::TapRightCommand,
            Action::DictateInEnglish => Shortcut::TapRightCommandRightShift,
            Action::EditSelection => Shortcut::RightOptionSpace,
        }
    }

    /// The action this shortcut triggers.
    pub fn action(self) -> Action {
        match self {
            Shortcut::TapRightCommand => Action::Dictate,
            Shortcut::TapRightCommandRightShift => Action::DictateInEnglish,
            Shortcut::RightOptionSpace => Action::EditSelection,
        }
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
/// no longer exist (any saved shortcut choice, now that the shortcuts are fixed, and the
/// on-device and local speech model choices) are ignored and new ones take defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
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
    fn each_action_has_its_own_right_hand_shortcut() {
        assert_eq!(
            Shortcut::for_action(Action::Dictate),
            Shortcut::TapRightCommand
        );
        assert_eq!(
            Shortcut::for_action(Action::DictateInEnglish),
            Shortcut::TapRightCommandRightShift
        );
        assert_eq!(
            Shortcut::for_action(Action::EditSelection),
            Shortcut::RightOptionSpace
        );
    }

    #[test]
    fn there_are_exactly_three_shortcuts_each_leading_back_to_its_action() {
        assert_eq!(Shortcut::ALL.len(), 3);
        for (shortcut, action) in Shortcut::ALL.into_iter().zip(Action::ALL) {
            assert_eq!(shortcut.action(), action);
            assert_eq!(Shortcut::for_action(action), shortcut);
        }
    }

    #[test]
    fn settings_round_trip_through_json() {
        let settings = Settings {
            speech: SpeechSettings {
                provider: SpeechProvider::OpenAi,
                model: "gpt-4o-mini-transcribe".into(),
            },
            assistant_model: "twil-8b".into(),
            ..Settings::default()
        };

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
    fn settings_with_saved_shortcut_choices_still_load() {
        let old = r#"{
            "shortcuts": {
                "dictate": "TapFn",
                "english": "ControlShiftSpace",
                "edit_selection": "OptionSpace"
            },
            "microphone": "USB Mic",
            "assistant_model": "twil-8b",
            "cleanup_enabled": false
        }"#;

        let s: Settings = serde_json::from_str(old).unwrap();

        assert_eq!(s.microphone.as_deref(), Some("USB Mic"));
        assert_eq!(s.assistant_model, "twil-8b");
        assert!(!s.cleanup_enabled);
        let saved_again = serde_json::to_string(&s).unwrap();
        assert!(!saved_again.contains("shortcuts"), "{saved_again}");
    }
}
