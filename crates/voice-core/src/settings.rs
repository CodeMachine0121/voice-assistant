use crate::PersonalDictionary;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A global keyboard shortcut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shortcut {
    /// Holding the right Option key alone.
    RightOption,
    /// Holding the right Command key alone.
    RightCommand,
    /// Holding the Fn key alone.
    Fn,
    /// Control + Option + Space.
    ControlOptionSpace,
    /// Control + Shift + Space.
    ControlShiftSpace,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub push_to_talk: Shortcut,
    pub hands_free: Shortcut,
    /// `None` = system default microphone.
    pub microphone: Option<String>,
    /// OpenAI-compatible base URL of the writing assistant (LM Studio default).
    pub assistant_url: String,
    /// Model identifier as listed by the writing assistant; empty = whatever is loaded.
    pub assistant_model: String,
    pub cleanup_enabled: bool,
    pub dictionary: PersonalDictionary,
    /// `None` = default location chosen by the speech adapter.
    pub speech_model_path: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            push_to_talk: Shortcut::RightOption,
            hands_free: Shortcut::ControlOptionSpace,
            microphone: None,
            assistant_url: "http://localhost:1234/v1".to_string(),
            assistant_model: String::new(),
            cleanup_enabled: true,
            dictionary: PersonalDictionary::default(),
            speech_model_path: None,
        }
    }
}
