//! Chinese script rule: Chinese is always written in Traditional characters (Taiwan).
use zhconv::{zhconv, Variant};

/// Converts any Chinese in `text` to Traditional characters as used in Taiwan.
/// Text in other languages is returned unchanged.
pub fn to_traditional(text: &str) -> String {
    zhconv(text, Variant::ZhTW)
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
    fn other_languages_are_unchanged() {
        for text in ["What time is it?", "ありがとうございます", "", "  "] {
            assert_eq!(to_traditional(text), text);
        }
    }
}
