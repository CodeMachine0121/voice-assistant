//! Rows for the History window.
use chrono::{DateTime, Local};
use voice_core::{Action, History};

pub const EMPTY_TEXT: &str = "No dictations yet.";
/// Longest stretch of the original text an Edit entry shows.
pub const ORIGINAL_PREVIEW_CHARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRow {
    pub time: String,
    pub action: Action,
    /// Text of the action chip.
    pub chip: &'static str,
    /// What the action wrote; Copy copies this.
    pub result: String,
    /// Edit: the spoken instruction.
    pub instruction: Option<String>,
    /// Edit: the selected text before the change, shortened for display.
    pub original: Option<String>,
    /// Dictate and English: the heard words, when they say something the result
    /// does not (otherwise the window shows the text only once).
    pub heard: Option<String>,
}

pub fn format_time(at: &DateTime<Local>) -> String {
    at.format("%Y-%m-%d %H:%M").to_string()
}

/// `text` cut to `max_chars` characters with an ellipsis when it is longer.
pub fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut short: String = text.chars().take(max_chars).collect();
    short.push('…');
    short
}

/// Newest first, as stored.
pub fn rows(history: &History) -> Vec<HistoryRow> {
    history
        .entries()
        .iter()
        .map(|e| {
            let edit = e.action == Action::EditSelection;
            HistoryRow {
                time: format_time(&e.at),
                action: e.action,
                chip: e.action.label(),
                result: e.polished.clone(),
                instruction: edit.then(|| e.raw.clone()),
                original: e
                    .original
                    .as_ref()
                    .filter(|_| edit)
                    .map(|o| truncate(o, ORIGINAL_PREVIEW_CHARS)),
                heard: (!edit && e.raw.trim() != e.polished.trim()).then(|| e.raw.clone()),
            }
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
            action: Action::Dictate,
            raw: raw.into(),
            original: None,
            polished: polished.into(),
        }
    }

    fn edit(instruction: &str, original: &str, result: &str) -> HistoryEntry {
        HistoryEntry {
            action: Action::EditSelection,
            original: Some(original.into()),
            ..entry(9, instruction, result)
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
        assert_eq!(rows[0].result, "p2");
        assert_eq!(rows[1].result, "p1");
    }

    #[test]
    fn dictate_row_shows_time_result_and_heard_words() {
        let history = History::from(vec![entry(9, "um hello", "Hello.")]);
        let row = &rows(&history)[0];
        assert_eq!(row.time, "2026-11-22 09:05");
        assert_eq!(row.chip, "Dictate");
        assert_eq!(row.result, "Hello.");
        assert_eq!(row.heard.as_deref(), Some("um hello"));
        assert_eq!(row.instruction, None);
        assert_eq!(row.original, None);
    }

    #[test]
    fn identical_heard_words_are_not_repeated() {
        let history = History::from(vec![entry(9, " Hello. ", "Hello.")]);
        assert_eq!(rows(&history)[0].heard, None);
    }

    #[test]
    fn an_english_entry_is_marked_english_and_shows_the_heard_words() {
        let mut e = entry(9, "我們開會", "We have a meeting.");
        e.action = Action::DictateInEnglish;
        let row = &rows(&History::from(vec![e]))[0];
        assert_eq!(row.chip, "English");
        assert_eq!(row.action, Action::DictateInEnglish);
        assert_eq!(row.heard.as_deref(), Some("我們開會"));
        assert_eq!(row.result, "We have a meeting.");
    }

    #[test]
    fn an_edit_entry_shows_instruction_original_and_result() {
        let history = History::from(vec![edit(
            "translate to English",
            "我們明天開會",
            "We have a meeting tomorrow.",
        )]);
        let row = &rows(&history)[0];
        assert_eq!(row.chip, "Edit");
        assert_eq!(row.instruction.as_deref(), Some("translate to English"));
        assert_eq!(row.original.as_deref(), Some("我們明天開會"));
        assert_eq!(row.result, "We have a meeting tomorrow.");
        assert_eq!(row.heard, None);
    }

    #[test]
    fn a_long_original_is_shortened_but_the_result_is_not() {
        let original = "字".repeat(300);
        let result = "x".repeat(300);
        let history = History::from(vec![edit("shorten", &original, &result)]);
        let row = &rows(&history)[0];
        let shown = row.original.as_deref().unwrap();
        assert_eq!(shown.chars().count(), ORIGINAL_PREVIEW_CHARS + 1);
        assert!(shown.ends_with('…'));
        assert_eq!(row.result, result);
    }

    #[test]
    fn an_original_of_exactly_the_limit_is_kept_whole() {
        let original = "a".repeat(ORIGINAL_PREVIEW_CHARS);
        assert_eq!(truncate(&original, ORIGINAL_PREVIEW_CHARS), original);
    }
}
