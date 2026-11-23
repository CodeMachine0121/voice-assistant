//! Ports: everything the dictation rules need from the outside world.
use crate::{History, PersonalDictionary, Problem, Recording, Settings, WritingTask};
use std::fmt;

/// Captures sound from the chosen microphone.
pub trait Recorder: Send {
    /// Begin capturing. Fails with `Problem::MicrophonePermission` when access is denied.
    fn start(&mut self) -> Result<(), Problem>;
    /// Stop capturing and return everything heard since `start`, at `rules::SAMPLE_RATE`.
    fn stop(&mut self) -> Recording;
    /// Stop capturing and throw the sound away.
    fn cancel(&mut self);
    /// Live input level in [0.0, 1.0] while capturing; 0.0 otherwise.
    fn level(&self) -> f32;
}

/// Why a recording could not be turned into words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscribeError {
    /// The speech model runs elsewhere (the local AI app) and could not be reached.
    Unreachable(String),
    /// The speech model was there but failed (e.g. model file missing or corrupt).
    Failed(String),
}

impl fmt::Display for TranscribeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TranscribeError::Unreachable(reason) => {
                write!(f, "speech model unreachable: {reason}")
            }
            TranscribeError::Failed(reason) => write!(f, "{reason}"),
        }
    }
}

/// Turns a recording into the heard words.
pub trait Transcriber: Send + Sync {
    fn transcribe(
        &self,
        recording: &Recording,
        dictionary: &PersonalDictionary,
    ) -> Result<String, TranscribeError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolishError {
    /// Not running, refused, or did not answer within `rules::ASSISTANT_TIMEOUT`.
    Unreachable(String),
    /// Answered, but with something unusable (e.g. empty text).
    InvalidResponse(String),
}

/// The writing assistant: carries out one `WritingTask` and returns the resulting text.
pub trait Polisher: Send + Sync {
    fn polish(
        &self,
        task: &WritingTask<'_>,
        dictionary: &PersonalDictionary,
    ) -> Result<String, PolishError>;
}

/// Reads the text selected in the front app without disturbing the clipboard.
pub trait SelectionReader: Send + Sync {
    /// `Ok(None)` = nothing selected or the app does not expose it.
    fn selected_text(&self) -> Result<Option<String>, Problem>;
}

/// Places text at the cursor of the front app, preserving the clipboard.
pub trait TextInserter: Send + Sync {
    /// On missing accessibility permission: leaves `text` on the clipboard and returns
    /// `Problem::AccessibilityPermission`.
    fn insert(&self, text: &str) -> Result<(), Problem>;
}

pub trait HistoryStore: Send + Sync {
    /// Missing or unreadable storage yields an empty history.
    fn load(&self) -> History;
    fn save(&self, history: &History) -> Result<(), String>;
}

pub trait SettingsStore: Send + Sync {
    /// Missing or unreadable storage yields `Settings::default()`.
    fn load(&self) -> Settings;
    fn save(&self, settings: &Settings) -> Result<(), String>;
}
