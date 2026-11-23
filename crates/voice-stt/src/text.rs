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

/// Joins the model's segments into the transcript, dropping segments made only of
/// non-speech markers — `[BLANK_AUDIO]`, `(music) (applause)`, `*applause*`, `♪ ♪ ♪` — or
/// of nothing. Silence therefore comes out as an empty string, while speech that merely
/// starts and ends with markers, such as `(笑) 好啊 (笑)`, is kept whole.
pub fn clean_transcript<S: AsRef<str>>(segments: &[S]) -> String {
    let joined: String = segments
        .iter()
        .map(AsRef::as_ref)
        .filter(|segment| !is_non_speech(segment))
        .collect();
    joined.trim().to_string()
}

/// Bracket pairs that wrap a description of non-speech.
const MARKERS: [(char, char); 6] = [
    ('[', ']'),
    ('(', ')'),
    ('（', '）'),
    ('【', '】'),
    ('♪', '♪'),
    ('*', '*'),
];

/// Whether the segment holds nothing but markers: they are stripped from the front one by
/// one, and any other text left over makes it speech.
fn is_non_speech(segment: &str) -> bool {
    let mut rest = segment.trim();
    while !rest.is_empty() {
        match strip_marker(rest) {
            Some(after) => rest = after.trim_start(),
            None => return false,
        }
    }
    true
}

/// `s` without the marker it starts with; `None` when it does not start with one.
fn strip_marker(s: &str) -> Option<&str> {
    MARKERS.iter().find_map(|&(open, close)| {
        let inside_and_rest = s.strip_prefix(open)?;
        match inside_and_rest.find(close) {
            // A nested opening bracket means this is not one simple marker.
            Some(end) if !inside_and_rest[..end].contains(open) => {
                Some(&inside_and_rest[end + close.len_utf8()..])
            }
            Some(_) => None,
            // A music note on its own stands for music; a lone `*` is just a character.
            None if open == '♪' => Some(inside_and_rest),
            None => None,
        }
    })
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
    fn a_transcript_made_only_of_markers_is_silence() {
        for markers in [
            "[Music] [Music]",
            "(music) (applause)",
            "♪ ♪ ♪",
            "♪",
            " [BLANK_AUDIO]  (music) *applause* ",
        ] {
            assert_eq!(clean_transcript(&[markers]), "", "{markers}");
        }
    }

    #[test]
    fn markers_around_and_between_speech_keep_the_whole_transcript() {
        assert_eq!(clean_transcript(&["(笑) 好啊 (笑)"]), "(笑) 好啊 (笑)");
        assert_eq!(
            clean_transcript(&["[Music] hello [Music]"]),
            "[Music] hello [Music]"
        );
        assert_eq!(clean_transcript(&["(a (b) c)"]), "(a (b) c)");
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
