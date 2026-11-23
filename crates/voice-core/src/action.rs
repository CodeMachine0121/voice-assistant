//! The user actions a recording can carry, from the shortcut that starts it to the
//! history entry it ends in.
use crate::rules::MAX_SELECTION_CHARS;
use crate::{Problem, SelectionReader};
use serde::{Deserialize, Serialize};

/// What the user wants done with the words they are about to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Action {
    /// Write what was said, cleaned up, in the language it was said in.
    #[default]
    Dictate,
    /// Write what was said, in English.
    DictateInEnglish,
    /// Apply the spoken instruction to the selected text and replace it.
    EditSelection,
}

impl Action {
    /// Every action, in the order they are presented to the user.
    pub const ALL: [Action; 3] = [
        Action::Dictate,
        Action::DictateInEnglish,
        Action::EditSelection,
    ];

    /// Short name shown on the indicator chip and in history.
    pub fn label(self) -> &'static str {
        match self {
            Action::Dictate => "Dictate",
            Action::DictateInEnglish => "English",
            Action::EditSelection => "Edit",
        }
    }

    /// One-line explanation shown next to the action's shortcut in Settings.
    pub fn description(self) -> &'static str {
        match self {
            Action::Dictate => "Write what you say",
            Action::DictateInEnglish => "Write what you say in English",
            Action::EditSelection => "Say what to do with the selected text",
        }
    }
}

/// An action ready to run: everything it needs was gathered before listening started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparedAction {
    Dictate,
    DictateInEnglish,
    /// `selection` is the text selected in the front app when the shortcut was pressed.
    EditSelection {
        selection: String,
    },
}

impl PreparedAction {
    /// Gets `action` ready to run. Only Edit selection reads the selection: it must not be
    /// empty and may hold at most `MAX_SELECTION_CHARS` characters (inclusive).
    pub fn prepare(action: Action, selection: &dyn SelectionReader) -> Result<Self, Problem> {
        match action {
            Action::Dictate => Ok(PreparedAction::Dictate),
            Action::DictateInEnglish => Ok(PreparedAction::DictateInEnglish),
            Action::EditSelection => match selection.selected_text()? {
                Some(text) if text.trim().is_empty() => Err(Problem::NoSelection),
                Some(text) if text.chars().count() > MAX_SELECTION_CHARS => {
                    Err(Problem::SelectionTooLong)
                }
                Some(text) => Ok(PreparedAction::EditSelection { selection: text }),
                None => Err(Problem::NoSelection),
            },
        }
    }

    pub fn action(&self) -> Action {
        match self {
            PreparedAction::Dictate => Action::Dictate,
            PreparedAction::DictateInEnglish => Action::DictateInEnglish,
            PreparedAction::EditSelection { .. } => Action::EditSelection,
        }
    }
}

/// What the writing assistant must do. Heard words and selected text are material to work
/// on, never instructions to the assistant; only `instruction` is the user's request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritingTask<'a> {
    /// Clean up the heard words, keeping their language.
    CleanUp { heard: &'a str },
    /// Clean up the heard words and write them in natural English.
    TranslateToEnglish { heard: &'a str },
    /// Apply the spoken `instruction` to `selection`.
    EditSelection {
        instruction: &'a str,
        selection: &'a str,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::FakeSelectionReader;

    #[test]
    fn labels_and_descriptions_read_as_shown_to_the_user() {
        let shown: Vec<_> = Action::ALL
            .iter()
            .map(|a| (a.label(), a.description()))
            .collect();

        assert_eq!(
            shown,
            [
                ("Dictate", "Write what you say"),
                ("English", "Write what you say in English"),
                ("Edit", "Say what to do with the selected text"),
            ]
        );
    }

    #[test]
    fn dictate_is_the_default_action() {
        assert_eq!(Action::default(), Action::Dictate);
    }

    #[test]
    fn dictating_never_reads_the_selection() {
        let reader = FakeSelectionReader::new(Ok(None));

        assert_eq!(
            PreparedAction::prepare(Action::Dictate, &*reader),
            Ok(PreparedAction::Dictate)
        );
        assert_eq!(
            PreparedAction::prepare(Action::DictateInEnglish, &*reader),
            Ok(PreparedAction::DictateInEnglish)
        );
        assert_eq!(reader.calls(), 0);
    }

    #[test]
    fn edit_selection_holds_the_selected_text() {
        let reader = FakeSelectionReader::new(Ok(Some("我們明天開會")));

        let prepared = PreparedAction::prepare(Action::EditSelection, &*reader);

        assert_eq!(
            prepared,
            Ok(PreparedAction::EditSelection {
                selection: "我們明天開會".into()
            })
        );
        assert_eq!(reader.calls(), 1);
    }

    #[test]
    fn a_selection_of_exactly_ten_thousand_characters_is_accepted() {
        // multi-byte characters: the limit counts characters, not bytes
        let text = "字".repeat(MAX_SELECTION_CHARS);
        let reader = FakeSelectionReader::new(Ok(Some(&text)));

        let prepared = PreparedAction::prepare(Action::EditSelection, &*reader);

        assert_eq!(
            prepared,
            Ok(PreparedAction::EditSelection { selection: text })
        );
    }

    #[test]
    fn a_selection_over_ten_thousand_characters_is_refused() {
        let text = "a".repeat(MAX_SELECTION_CHARS + 1);
        let reader = FakeSelectionReader::new(Ok(Some(&text)));

        assert_eq!(
            PreparedAction::prepare(Action::EditSelection, &*reader),
            Err(Problem::SelectionTooLong)
        );
    }

    #[test]
    fn nothing_selected_is_refused() {
        let reader = FakeSelectionReader::new(Ok(None));

        assert_eq!(
            PreparedAction::prepare(Action::EditSelection, &*reader),
            Err(Problem::NoSelection)
        );
    }

    #[test]
    fn a_blank_selection_counts_as_nothing_selected() {
        let reader = FakeSelectionReader::new(Ok(Some(" \n\t")));

        assert_eq!(
            PreparedAction::prepare(Action::EditSelection, &*reader),
            Err(Problem::NoSelection)
        );
    }

    #[test]
    fn a_failure_to_read_the_selection_is_passed_on() {
        let reader = FakeSelectionReader::new(Err(Problem::AccessibilityPermission));

        assert_eq!(
            PreparedAction::prepare(Action::EditSelection, &*reader),
            Err(Problem::AccessibilityPermission)
        );
    }

    #[test]
    fn a_prepared_action_knows_its_action() {
        let edit = PreparedAction::EditSelection {
            selection: "x".into(),
        };

        assert_eq!(PreparedAction::Dictate.action(), Action::Dictate);
        assert_eq!(
            PreparedAction::DictateInEnglish.action(),
            Action::DictateInEnglish
        );
        assert_eq!(edit.action(), Action::EditSelection);
    }
}
