use crate::{Action, SpeechProvider, TranscribeError};
use std::fmt;

/// A problem the user must be told about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    NoSpeech,
    /// The writing assistant could not be reached while running this action.
    AssistantUnreachable(Action),
    /// The speech provider could not turn the recording into words.
    Speech {
        provider: SpeechProvider,
        failure: SpeechFailure,
    },
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
            Problem::Speech { provider, failure } => failure.describe(*provider, f),
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

/// Why the speech provider could not turn a recording into words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechFailure {
    /// No connection, no answer within the time limit, or trouble on the provider's side.
    Unreachable,
    /// No key for the speech provider is saved or set up on this Mac.
    KeyMissing,
    /// A key is saved, but it could not be read from the Keychain.
    KeyUnreadable,
    /// The speech provider does not accept the key.
    KeyRejected,
    /// Too many requests, or the provider account has no credit left.
    LimitReached,
    /// The speech provider refused the recording or answered with nothing usable.
    NotTranscribed,
}

impl SpeechFailure {
    /// The message the user sees, naming the provider.
    fn describe(self, provider: SpeechProvider, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = provider.label();
        match self {
            SpeechFailure::Unreachable => {
                write!(f, "{name} not reachable — check your internet connection")
            }
            SpeechFailure::KeyMissing => {
                write!(f, "{name} key missing — add it in Settings › AI models")
            }
            SpeechFailure::KeyUnreadable => write!(
                f,
                "Couldn't read the {name} key from the Keychain — allow access or save it again in Settings › AI models"
            ),
            SpeechFailure::KeyRejected => {
                write!(
                    f,
                    "{name} rejected the key — check it in Settings › AI models"
                )
            }
            SpeechFailure::LimitReached => write!(
                f,
                "{name} limit reached — try again later or check your {name} account"
            ),
            SpeechFailure::NotTranscribed => {
                write!(f, "{name} couldn't turn the recording into words")
            }
        }
    }
}

impl From<&TranscribeError> for SpeechFailure {
    fn from(error: &TranscribeError) -> Self {
        match error {
            TranscribeError::Unreachable(_) => SpeechFailure::Unreachable,
            TranscribeError::KeyMissing => SpeechFailure::KeyMissing,
            TranscribeError::KeyUnreadable => SpeechFailure::KeyUnreadable,
            TranscribeError::KeyRejected => SpeechFailure::KeyRejected,
            TranscribeError::LimitReached => SpeechFailure::LimitReached,
            TranscribeError::Failed(_) => SpeechFailure::NotTranscribed,
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

    fn openai(failure: SpeechFailure) -> Problem {
        Problem::Speech {
            provider: SpeechProvider::OpenAi,
            failure,
        }
    }

    #[test]
    fn transcribe_errors_become_speech_failures() {
        let cases = [
            (
                TranscribeError::Unreachable("x".into()),
                SpeechFailure::Unreachable,
            ),
            (TranscribeError::KeyMissing, SpeechFailure::KeyMissing),
            (TranscribeError::KeyUnreadable, SpeechFailure::KeyUnreadable),
            (TranscribeError::KeyRejected, SpeechFailure::KeyRejected),
            (TranscribeError::LimitReached, SpeechFailure::LimitReached),
            (
                TranscribeError::Failed("x".into()),
                SpeechFailure::NotTranscribed,
            ),
        ];

        for (error, failure) in cases {
            assert_eq!(SpeechFailure::from(&error), failure, "{error}");
        }
    }

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
                openai(SpeechFailure::Unreachable),
                "OpenAI not reachable — check your internet connection",
            ),
            (
                openai(SpeechFailure::KeyMissing),
                "OpenAI key missing — add it in Settings › AI models",
            ),
            (
                openai(SpeechFailure::KeyUnreadable),
                "Couldn't read the OpenAI key from the Keychain — allow access or save it again in Settings › AI models",
            ),
            (
                openai(SpeechFailure::KeyRejected),
                "OpenAI rejected the key — check it in Settings › AI models",
            ),
            (
                openai(SpeechFailure::LimitReached),
                "OpenAI limit reached — try again later or check your OpenAI account",
            ),
            (
                openai(SpeechFailure::NotTranscribed),
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
