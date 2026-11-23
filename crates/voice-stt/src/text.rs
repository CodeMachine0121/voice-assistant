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

/// Joins the model's segments into the transcript, dropping segments that are exactly one
/// non-speech marker — `[BLANK_AUDIO]`, `(music)`, `*applause*`, `♪ ... ♪` — or hold
/// nothing. Silence therefore comes out as an empty string, while speech that merely starts
/// and ends with brackets, such as `(笑) 好啊 (笑)`, is kept.
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
    // One marker: a single bracket pair around the whole segment, with no other bracket of
    // that kind inside (so no text can sit outside the pair).
    let wrapped = |open: char, close: char| {
        s.strip_prefix(open)
            .and_then(|rest| rest.strip_suffix(close))
            .is_some_and(|inside| !inside.contains(open) && !inside.contains(close))
    };
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
    fn speech_that_starts_and_ends_with_markers_is_kept() {
        assert_eq!(clean_transcript(&["(笑) 好啊 (笑)"]), "(笑) 好啊 (笑)");
        assert_eq!(
            clean_transcript(&["[laughs] okay [laughs]"]),
            "[laughs] okay [laughs]"
        );
        assert_eq!(
            clean_transcript(&["*sigh* fine then *sigh*"]),
            "*sigh* fine then *sigh*"
        );
        assert_eq!(clean_transcript(&["♪ la ♪ hi ♪ la ♪"]), "♪ la ♪ hi ♪ la ♪");
    }

    #[test]
    fn a_whole_transcript_that_is_one_marker_is_silence() {
        for marker in ["[BLANK_AUDIO]", " (music) ", "（笑）", "*applause*"] {
            assert_eq!(clean_transcript(&[marker]), "", "{marker}");
        }
    }

    #[test]
    fn a_lone_asterisk_is_not_a_marker() {
        assert_eq!(clean_transcript(&["*"]), "*");
    }
}
