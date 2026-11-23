//! Post-processing of the assistant's answer.

/// Which of the prompt's tags the model may echo back around its answer.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EchoedTags {
    /// Tags that wrapped the content being worked on: only the tag text is removed.
    pub(crate) unwrap: &'static [&'static str],
    /// Tags that held input only: the whole block is removed.
    pub(crate) drop: &'static [&'static str],
}

/// Removes reasoning blocks, echoed prompt tags, code fences and surrounding quotes,
/// then trims.
pub(crate) fn clean_answer(answer: &str, echoed: EchoedTags) -> String {
    let mut text = strip_block(answer, "think");
    for name in echoed.drop {
        text = strip_block(&text, name);
    }
    for name in echoed.unwrap {
        text = text
            .replace(&format!("<{name}>"), "")
            .replace(&format!("</{name}>"), "");
    }
    let text = strip_fence(text.trim());
    strip_quotes(text.trim()).trim().to_string()
}

/// Removes every `<name>…</name>` block; an unterminated block swallows the rest.
fn strip_block(text: &str, name: &str) -> String {
    let (open, close) = (format!("<{name}>"), format!("</{name}>"));
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(&open) {
        out.push_str(&rest[..start]);
        match rest[start..].find(&close) {
            Some(end) => rest = &rest[start + end + close.len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn strip_fence(text: &str) -> &str {
    let Some(inner) = text.strip_prefix("```").and_then(|t| t.strip_suffix("```")) else {
        return text;
    };
    // Drop an optional language tag on the opening line.
    match inner.split_once('\n') {
        Some((_, body)) => body,
        None => inner,
    }
}

fn strip_quotes(text: &str) -> &str {
    const PAIRS: [(char, char); 4] = [('"', '"'), ('\'', '\''), ('“', '”'), ('「', '」')];
    for (open, close) in PAIRS {
        if let Some(inner) = text.strip_prefix(open).and_then(|t| t.strip_suffix(close)) {
            return inner;
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::EchoedTags;

    fn clean_answer(answer: &str) -> String {
        super::clean_answer(answer, EchoedTags::default())
    }

    #[test]
    fn trims_plain_text() {
        assert_eq!(clean_answer("  Hello.\n"), "Hello.");
    }

    #[test]
    fn removes_think_blocks() {
        assert_eq!(
            clean_answer("<think>hmm\nlet me see</think>\nHello."),
            "Hello."
        );
        assert_eq!(clean_answer("A<think>x</think> B<think>y</think>C"), "A BC");
    }

    #[test]
    fn unterminated_think_drops_the_rest() {
        assert_eq!(clean_answer("Hello.<think>never ends"), "Hello.");
    }

    #[test]
    fn removes_code_fences_with_and_without_language() {
        assert_eq!(clean_answer("```\nHello.\n```"), "Hello.");
        assert_eq!(clean_answer("```text\nHello.\n```"), "Hello.");
        assert_eq!(clean_answer("```Hello.```"), "Hello.");
    }

    #[test]
    fn removes_surrounding_quotes() {
        assert_eq!(clean_answer("\"Hello.\""), "Hello.");
        assert_eq!(clean_answer("'Hello.'"), "Hello.");
        assert_eq!(clean_answer("“Hello.”"), "Hello.");
        assert_eq!(clean_answer("「你好。」"), "你好。");
    }

    #[test]
    fn keeps_inner_and_unbalanced_quotes() {
        assert_eq!(
            clean_answer("He said \"hi\" today."),
            "He said \"hi\" today."
        );
        assert_eq!(clean_answer("\"Hello."), "\"Hello.");
    }

    #[test]
    fn only_reasoning_leaves_nothing() {
        assert_eq!(clean_answer("<think>x</think>"), "");
    }

    #[test]
    fn echoed_wrapping_tags_are_removed_but_their_text_kept() {
        let echoed = EchoedTags {
            unwrap: &["selection"],
            drop: &[],
        };
        assert_eq!(
            super::clean_answer("<selection>We meet tomorrow.</selection>", echoed),
            "We meet tomorrow."
        );
    }

    #[test]
    fn echoed_input_blocks_are_removed_whole() {
        let echoed = EchoedTags {
            unwrap: &["selection"],
            drop: &["instruction"],
        };
        assert_eq!(
            super::clean_answer(
                "<instruction>translate</instruction>\n<selection>We meet.</selection>",
                echoed
            ),
            "We meet."
        );
    }

    #[test]
    fn tags_of_other_tasks_are_left_alone() {
        let echoed = EchoedTags {
            unwrap: &["transcript"],
            drop: &[],
        };
        assert_eq!(
            super::clean_answer("Use <selection> here.", echoed),
            "Use <selection> here."
        );
    }
}
