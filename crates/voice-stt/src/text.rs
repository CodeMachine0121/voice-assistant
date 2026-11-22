//! Pure text rules around the speech model: what to prime it with and how to clean its output.
use voice_core::PersonalDictionary;

/// The model is shown the user's own words first, which biases it toward spelling them
/// that way. `None` when the dictionary is empty, so nothing is primed.
pub fn initial_prompt(dictionary: &PersonalDictionary) -> Option<String> {
    if dictionary.is_empty() {
        None
    } else {
        Some(dictionary.words().join(", "))
    }
}

/// Joins the model's segments into the transcript, dropping segments that only describe
/// non-speech — `[BLANK_AUDIO]`, `(music)`, `*applause*`, `♪ ... ♪` — or hold nothing.
/// Silence therefore comes out as an empty string.
pub fn clean_transcript<S: AsRef<str>>(segments: &[S]) -> String {
    let joined: String = segments
        .iter()
        .map(AsRef::as_ref)
        .filter(|segment| !is_non_speech(segment))
        .collect();
    joined.trim().to_string()
}

fn is_non_speech(segment: &str) -> bool {
    let s = segment.trim();
    if s.is_empty() {
        return true;
    }
    let wrapped = |open: char, close: char| s.starts_with(open) && s.ends_with(close);
    wrapped('[', ']')
        || wrapped('(', ')')
        || wrapped('（', '）')
        || wrapped('【', '】')
        || wrapped('♪', '♪')
        || (s.len() > 1 && wrapped('*', '*'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_lists_dictionary_words_in_order() {
        let dictionary = PersonalDictionary::from_words(["Cafler", "Kubernetes"]);
        assert_eq!(
            initial_prompt(&dictionary),
            Some("Cafler, Kubernetes".to_string())
        );
    }

    #[test]
    fn prompt_of_a_single_word_is_that_word() {
        let dictionary = PersonalDictionary::from_words(["Cafler"]);
        assert_eq!(initial_prompt(&dictionary), Some("Cafler".to_string()));
    }

    #[test]
    fn prompt_is_omitted_for_an_empty_dictionary() {
        assert_eq!(initial_prompt(&PersonalDictionary::default()), None);
    }

    #[test]
    fn segments_are_joined_and_trimmed() {
        assert_eq!(clean_transcript(&[" Hello", " world. "]), "Hello world.");
    }

    #[test]
    fn chinese_segments_join_without_added_spaces() {
        assert_eq!(clean_transcript(&["你好", "世界"]), "你好世界");
    }

    #[test]
    fn blank_audio_marker_alone_is_silence() {
        assert_eq!(clean_transcript(&[" [BLANK_AUDIO]"]), "");
    }

    #[test]
    fn no_segments_is_silence() {
        assert_eq!(clean_transcript::<&str>(&[]), "");
    }

    #[test]
    fn whitespace_only_segments_are_silence() {
        assert_eq!(clean_transcript(&["  ", "\n"]), "");
    }

    #[test]
    fn bracketed_markers_are_removed_between_speech() {
        assert_eq!(
            clean_transcript(&[" Hello", " (music)", " [BLANK_AUDIO]", " there."]),
            "Hello there."
        );
    }

    #[test]
    fn other_marker_styles_are_removed() {
        for marker in [
            "*applause*",
            " ♪ la la ♪",
            "（音樂）",
            "【掌聲】",
            "(speaking in foreign language)",
        ] {
            assert_eq!(clean_transcript(&[marker]), "", "{marker}");
        }
    }

    #[test]
    fn brackets_inside_real_speech_are_kept() {
        assert_eq!(
            clean_transcript(&[" Call me (maybe) tomorrow."]),
            "Call me (maybe) tomorrow."
        );
    }

    #[test]
    fn a_lone_asterisk_is_not_a_marker() {
        assert_eq!(clean_transcript(&["*"]), "*");
    }
}
