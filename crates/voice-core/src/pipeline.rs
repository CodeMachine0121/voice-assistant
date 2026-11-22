//! Finishing a dictation: speech check → transcribe → polish (or skip) → insert → history.
use crate::{
    HistoryEntry, HistoryStore, Polisher, Recording, Settings, Status, TextInserter, Transcriber,
};
use std::sync::Arc;

/// Result of processing one recording.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// Final status to show (Done, or Problem(..)).
    pub status: Status,
    /// The history entry recorded, if any.
    pub entry: Option<HistoryEntry>,
}

pub struct DictationPipeline {
    // implementation-defined
}

impl DictationPipeline {
    pub fn new(
        transcriber: Arc<dyn Transcriber>,
        polisher: Arc<dyn Polisher>,
        inserter: Arc<dyn TextInserter>,
        history: Arc<dyn HistoryStore>,
    ) -> Self {
        todo!()
    }

    pub fn process(&self, recording: Recording, settings: &Settings) -> Outcome {
        todo!()
    }
}
