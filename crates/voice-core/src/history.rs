use crate::rules::HISTORY_LIMIT;
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
        self.entries.insert(0, entry);
        self.entries.truncate(HISTORY_LIMIT);
    }

    /// Newest first.
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }
}

impl From<Vec<HistoryEntry>> for History {
    /// `entries` must already be newest first; anything beyond the limit is dropped.
    fn from(mut entries: Vec<HistoryEntry>) -> Self {
        entries.truncate(HISTORY_LIMIT);
        Self { entries }
    }
}

impl From<History> for Vec<HistoryEntry> {
    fn from(h: History) -> Self {
        h.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn entry(n: usize) -> HistoryEntry {
        HistoryEntry {
            at: Local.timestamp_opt(1_700_000_000 + n as i64, 0).unwrap(),
            raw: format!("raw {n}"),
            polished: format!("Polished {n}."),
        }
    }

    fn polished(history: &History) -> Vec<&str> {
        history
            .entries()
            .iter()
            .map(|e| e.polished.as_str())
            .collect()
    }

    #[test]
    fn lists_finished_dictations_newest_first() {
        let mut history = History::default();

        history.record(entry(1));
        history.record(entry(2));
        history.record(entry(3));

        assert_eq!(history.entries(), [entry(3), entry(2), entry(1)]);
    }

    #[test]
    fn keeps_at_most_200_entries_dropping_the_oldest() {
        // newest first: entry(200) .. entry(1)
        let mut history = History::from((1..=200).rev().map(entry).collect::<Vec<_>>());
        assert_eq!(history.entries().len(), 200);

        history.record(entry(201));

        assert_eq!(history.entries().len(), 200);
        assert_eq!(history.entries()[0], entry(201));
        assert_eq!(history.entries()[199], entry(2));
        assert!(!history.entries().contains(&entry(1)));
    }

    #[test]
    fn from_vec_keeps_the_newest_200() {
        let history = History::from((1..=250).rev().map(entry).collect::<Vec<_>>());

        assert_eq!(history.entries().len(), 200);
        assert_eq!(history.entries()[0], entry(250));
        assert_eq!(history.entries()[199], entry(51));
    }

    #[test]
    fn from_vec_keeps_short_lists_unchanged() {
        let history = History::from(vec![entry(2), entry(1)]);

        assert_eq!(polished(&history), ["Polished 2.", "Polished 1."]);
    }

    #[test]
    fn json_round_trip_is_a_plain_array_and_enforces_the_limit() {
        let many: Vec<HistoryEntry> = (1..=201).rev().map(entry).collect();
        let json = serde_json::to_string(&many).unwrap();

        let history: History = serde_json::from_str(&json).unwrap();
        let back = serde_json::to_value(&history).unwrap();

        assert_eq!(history.entries().len(), 200);
        assert_eq!(back.as_array().map(Vec::len), Some(200));
        assert_eq!(Vec::<HistoryEntry>::from(history)[0], entry(201));
    }
}
