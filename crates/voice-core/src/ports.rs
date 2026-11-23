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
    /// The speech provider could not be reached: no connection, no answer in time,
    /// or the provider reports trouble on its side.
    Unreachable(String),
    /// No key for the speech provider is available.
    KeyMissing,
    /// The speech provider does not accept the key.
    KeyRejected,
    /// The speech provider refuses more requests for now (rate limit or no credit left).
    LimitReached,
    /// The speech provider answered, but refused the recording or gave nothing usable.
    Failed(String),
}

impl fmt::Display for TranscribeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TranscribeError::Unreachable(reason) => {
                write!(f, "speech provider unreachable: {reason}")
            }
            TranscribeError::KeyMissing => write!(f, "no key for the speech provider"),
            TranscribeError::KeyRejected => write!(f, "speech provider rejected the key"),
            TranscribeError::LimitReached => write!(f, "speech provider limit reached"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcribe_errors_read_as_log_lines() {
        let cases = [
            (
                TranscribeError::Unreachable("timed out".into()),
                "speech provider unreachable: timed out",
            ),
            (
                TranscribeError::KeyMissing,
                "no key for the speech provider",
            ),
            (
                TranscribeError::KeyRejected,
                "speech provider rejected the key",
            ),
            (
                TranscribeError::LimitReached,
                "speech provider limit reached",
            ),
            (
                TranscribeError::Failed("unknown model".into()),
                "unknown model",
            ),
        ];

        for (error, line) in cases {
            assert_eq!(error.to_string(), line);
        }
    }
}
