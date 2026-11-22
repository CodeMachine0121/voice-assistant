//! Ports: everything the dictation rules need from the outside world.
use crate::{History, PersonalDictionary, Problem, Recording, Settings};

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

/// Turns a recording into the raw transcript, on device.
pub trait Transcriber: Send + Sync {
    fn transcribe(
        &self,
        recording: &Recording,
        dictionary: &PersonalDictionary,
    ) -> Result<String, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolishError {
    /// Not running, refused, or did not answer within `rules::ASSISTANT_TIMEOUT`.
    Unreachable(String),
    /// Answered, but with something unusable (e.g. empty text).
    InvalidResponse(String),
}

/// The writing assistant: rewrites a raw transcript into polished text.
pub trait Polisher: Send + Sync {
    fn polish(&self, raw: &str, dictionary: &PersonalDictionary) -> Result<String, PolishError>;
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
