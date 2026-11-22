//! Rows for the History window.
use chrono::{DateTime, Local};
use voice_core::History;

pub const EMPTY_TEXT: &str = "No dictations yet.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRow {
    pub time: String,
    pub polished: String,
    pub raw: String,
    /// Whether the raw transcript says something the polished text does not
    /// (otherwise the window shows it only once).
    pub raw_differs: bool,
}

pub fn format_time(at: &DateTime<Local>) -> String {
    at.format("%Y-%m-%d %H:%M").to_string()
}

/// Newest first, as stored.
pub fn rows(history: &History) -> Vec<HistoryRow> {
    history
        .entries()
        .iter()
        .map(|e| HistoryRow {
            time: format_time(&e.at),
            polished: e.polished.clone(),
            raw: e.raw.clone(),
            raw_differs: e.raw.trim() != e.polished.trim(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use voice_core::HistoryEntry;

    fn entry(hour: u32, raw: &str, polished: &str) -> HistoryEntry {
        HistoryEntry {
            at: Local.with_ymd_and_hms(2026, 11, 22, hour, 5, 0).unwrap(),
            raw: raw.into(),
            polished: polished.into(),
        }
    }

    #[test]
    fn empty_history_has_no_rows() {
        assert!(rows(&History::default()).is_empty());
        assert_eq!(EMPTY_TEXT, "No dictations yet.");
    }

    #[test]
    fn rows_keep_the_stored_newest_first_order() {
        let history = History::from(vec![entry(10, "r2", "p2"), entry(9, "r1", "p1")]);
        let rows = rows(&history);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].polished, "p2");
        assert_eq!(rows[1].polished, "p1");
    }

    #[test]
    fn row_shows_time_polished_and_raw() {
        let history = History::from(vec![entry(9, "um hello", "Hello.")]);
        let row = &rows(&history)[0];
        assert_eq!(row.time, "2026-11-22 09:05");
        assert_eq!(row.polished, "Hello.");
        assert_eq!(row.raw, "um hello");
        assert!(row.raw_differs);
    }

    #[test]
    fn identical_raw_is_flagged_as_not_different() {
        let history = History::from(vec![entry(9, " Hello. ", "Hello.")]);
        assert!(!rows(&history)[0].raw_differs);
    }
}
