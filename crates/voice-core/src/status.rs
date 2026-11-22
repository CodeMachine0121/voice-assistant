use std::fmt;

/// A problem the user must be told about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    NoSpeech,
    AssistantUnreachable,
    MicrophonePermission,
    AccessibilityPermission,
    Other(String),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::NoSpeech => write!(f, "No speech heard"),
            Problem::AssistantUnreachable => write!(f, "Cleanup skipped – assistant unreachable"),
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

/// What the user sees about the current dictation.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Idle,
    /// `level` is the live input level in [0.0, 1.0].
    Listening {
        level: f32,
    },
    Working,
    Done,
    Cancelled,
    Problem(Problem),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problems_read_as_the_messages_shown_to_the_user() {
        let cases = [
            (Problem::NoSpeech, "No speech heard"),
            (
                Problem::AssistantUnreachable,
                "Cleanup skipped – assistant unreachable",
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
}
