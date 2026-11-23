//! Dictation domain: business rules and ports. No platform or network code lives here.

pub mod action;
pub mod controller;
pub mod dictionary;
pub mod engine;
#[cfg(test)]
mod fakes;
pub mod history;
pub mod pipeline;
pub mod ports;
pub mod recording;
pub mod rules;
pub mod script;
pub mod settings;
pub mod speech;
pub mod status;

pub use action::{Action, PreparedAction, WritingTask};
pub use controller::{Command, ControllerEvent, SessionController, ShortcutEvent};
pub use dictionary::PersonalDictionary;
pub use engine::{Engine, EngineDeps, EngineHandle};
pub use history::{History, HistoryEntry};
pub use pipeline::{DictationPipeline, Outcome};
pub use ports::{
    HistoryStore, PolishError, Polisher, Recorder, SelectionReader, SettingsStore, TextInserter,
    TranscribeError, Transcriber,
};
pub use recording::Recording;
pub use rules::MAX_SELECTION_CHARS;
pub use settings::{ActionShortcuts, Settings, Shortcut, SpeechSource};
pub use status::{Problem, Status};
