use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub at: DateTime<Local>,
    pub raw: String,
    pub polished: String,
}

/// Finished dictations, newest first, at most `rules::HISTORY_LIMIT`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "Vec<HistoryEntry>", into = "Vec<HistoryEntry>")]
pub struct History {
    entries: Vec<HistoryEntry>,
}

impl History {
    /// Puts `entry` first; drops the oldest beyond the limit.
    pub fn record(&mut self, entry: HistoryEntry) {
        todo!()
    }

    /// Newest first.
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }
}

impl From<Vec<HistoryEntry>> for History {
    fn from(entries: Vec<HistoryEntry>) -> Self {
        todo!("enforce invariant")
    }
}

impl From<History> for Vec<HistoryEntry> {
    fn from(h: History) -> Self {
        h.entries
    }
}
