//! Chinese script rule: Chinese is always written in Traditional characters.
use zhconv::{zhconv, Variant};

/// Converts Simplified Chinese characters in `text` to Traditional ones.
///
/// Only characters change, never words: "打印機" stays as said rather than becoming the
/// Taiwanese "印表機" (word choice is the writing assistant's job). Text containing
/// Japanese kana is returned unchanged so Japanese kanji are not rewritten.
pub fn to_traditional(text: &str) -> String {
    if text.chars().any(is_kana) {
        return text.to_string();
    }
    zhconv(text, Variant::ZhHant)
}

fn is_kana(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplified_chinese_becomes_traditional() {
        assert_eq!(to_traditional("我们开会"), "我們開會");
    }

    #[test]
    fn traditional_chinese_is_kept() {
        assert_eq!(
            to_traditional("我們明天下午三點開會。"),
            "我們明天下午三點開會。"
        );
    }

    #[test]
    fn mixed_chinese_and_english_keeps_the_english() {
        assert_eq!(
            to_traditional("我们 review 一下 PR。"),
            "我們 review 一下 PR。"
        );
    }

    #[test]
    fn words_are_kept_as_said() {
        assert_eq!(to_traditional("打印机"), "打印機");
    }

    #[test]
    fn japanese_is_left_alone() {
        assert_eq!(to_traditional("会議は三時からです"), "会議は三時からです");
    }

    #[test]
    fn other_languages_are_unchanged() {
        for text in ["What time is it?", "ありがとうございます", "", "  "] {
            assert_eq!(to_traditional(text), text);
        }
    }
}
