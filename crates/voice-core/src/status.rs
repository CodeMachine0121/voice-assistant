use crate::{Action, SpeechProvider};
use std::fmt;

/// A problem the user must be told about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    NoSpeech,
    /// The writing assistant could not be reached while running this action.
    AssistantUnreachable(Action),
    /// No connection, no answer within the time limit, or trouble on the provider's side.
    SpeechUnreachable(SpeechProvider),
    /// No key for the speech provider is saved or set up on this Mac.
    SpeechKeyMissing(SpeechProvider),
    /// The speech provider does not accept the key.
    SpeechKeyRejected(SpeechProvider),
    /// Too many requests, or the provider account has no credit left.
    SpeechLimitReached(SpeechProvider),
    /// The speech provider refused the recording or answered with nothing usable.
    SpeechNotTranscribed(SpeechProvider),
    /// The writing assistant answered an Edit selection with nothing usable.
    InstructionNotCarriedOut,
    /// Edit selection was started with nothing selected.
    NoSelection,
    /// Edit selection was started with more than `rules::MAX_SELECTION_CHARS` selected.
    SelectionTooLong,
    MicrophonePermission,
    AccessibilityPermission,
    Other(String),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::NoSpeech => write!(f, "No speech heard"),
            Problem::AssistantUnreachable(Action::Dictate) => {
                write!(f, "AI cleanup skipped — writing assistant not reachable")
            }
            Problem::AssistantUnreachable(Action::DictateInEnglish) => {
                write!(f, "Translation skipped — writing assistant not reachable")
            }
            Problem::AssistantUnreachable(Action::EditSelection) => write!(
                f,
                "Couldn't run the instruction — writing assistant not reachable"
            ),
            Problem::InstructionNotCarriedOut => write!(f, "Couldn't run the instruction"),
            Problem::SpeechUnreachable(p) => write!(
                f,
                "{} not reachable — check your internet connection",
                p.label()
            ),
            Problem::SpeechKeyMissing(p) => write!(
                f,
                "{} key missing — add it in Settings › AI models",
                p.label()
            ),
            Problem::SpeechKeyRejected(p) => write!(
                f,
                "{} rejected the key — check it in Settings › AI models",
                p.label()
            ),
            Problem::SpeechLimitReached(p) => write!(
                f,
                "{0} limit reached — try again later or check your {0} account",
                p.label()
            ),
            Problem::SpeechNotTranscribed(p) => {
                write!(f, "{} couldn't turn the recording into words", p.label())
            }
            Problem::NoSelection => {
                write!(f, "Select text first, then start Edit selection again")
            }
            Problem::SelectionTooLong => {
                write!(f, "Selection too long — select at most 10,000 characters")
            }
            Problem::MicrophonePermission => write!(
                f,
                "Microphone access needed – System Settings › Privacy & Security › Microphone"
            ),
            Problem::AccessibilityPermission => write!(
                f,
                "Accessibility access needed – System Settings › Privacy & Security › Accessibility"
            ),
            Problem::Other(msg) => write!(f, "{msg}"),
        }
    }
}

/// What the user sees about the current recording.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Idle,
    /// `level` is the live input level in [0.0, 1.0].
    Listening {
        action: Action,
        level: f32,
    },
    /// The recording is being turned into text for this action.
    Working(Action),
    Done,
    Cancelled,
    Problem(Problem),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::MAX_SELECTION_CHARS;

    #[test]
    fn problems_read_as_the_messages_shown_to_the_user() {
        let cases = [
            (Problem::NoSpeech, "No speech heard"),
            (
                Problem::AssistantUnreachable(Action::Dictate),
                "AI cleanup skipped — writing assistant not reachable",
            ),
            (
                Problem::AssistantUnreachable(Action::DictateInEnglish),
                "Translation skipped — writing assistant not reachable",
            ),
            (
                Problem::AssistantUnreachable(Action::EditSelection),
                "Couldn't run the instruction — writing assistant not reachable",
            ),
            (
                Problem::SpeechUnreachable(SpeechProvider::OpenAi),
                "OpenAI not reachable — check your internet connection",
            ),
            (
                Problem::SpeechKeyMissing(SpeechProvider::OpenAi),
                "OpenAI key missing — add it in Settings › AI models",
            ),
            (
                Problem::SpeechKeyRejected(SpeechProvider::OpenAi),
                "OpenAI rejected the key — check it in Settings › AI models",
            ),
            (
                Problem::SpeechLimitReached(SpeechProvider::OpenAi),
                "OpenAI limit reached — try again later or check your OpenAI account",
            ),
            (
                Problem::SpeechNotTranscribed(SpeechProvider::OpenAi),
                "OpenAI couldn't turn the recording into words",
            ),
            (
                Problem::NoSelection,
                "Select text first, then start Edit selection again",
            ),
            (
                Problem::SelectionTooLong,
                "Selection too long — select at most 10,000 characters",
            ),
            (
                Problem::MicrophonePermission,
                "Microphone access needed – System Settings › Privacy & Security › Microphone",
            ),
            (
                Problem::AccessibilityPermission,
                "Accessibility access needed – System Settings › Privacy & Security › Accessibility",
            ),
            (Problem::Other("model missing".into()), "model missing"),
        ];

        for (problem, message) in cases {
            assert_eq!(problem.to_string(), message);
        }
    }

    #[test]
    fn the_too_long_message_names_the_actual_limit() {
        assert_eq!(MAX_SELECTION_CHARS, 10_000);
    }
}
