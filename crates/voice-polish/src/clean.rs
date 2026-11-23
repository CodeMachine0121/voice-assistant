//! Post-processing of the assistant's answer.

/// Which of the prompt's tags the model may echo back around its answer.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EchoedTags {
    /// Tags that wrapped the content being worked on: a pair wrapping the whole answer
    /// is removed, keeping its content.
    pub(crate) unwrap: &'static [&'static str],
    /// Tags that held input only: a block of one leading the answer is removed whole.
    pub(crate) drop: &'static [&'static str],
}

/// Removes reasoning blocks, echoed prompt tags, code fences and surrounding quotes,
/// then trims. Tags inside the answer's content are kept: they may be part of the text.
pub(crate) fn clean_answer(answer: &str, echoed: EchoedTags) -> String {
    let thought_free = strip_block(answer, "think");
    let mut text = thought_free.trim();
    for name in echoed.drop {
        text = strip_leading_block(text, name).trim();
    }
    for name in echoed.unwrap {
        text = strip_wrapping_pair(text, name).trim();
    }
    let text = strip_fence(text);
    strip_quotes(text.trim()).trim().to_string()
}

/// Removes a `<name>…</name>` block that starts the text.
fn strip_leading_block<'a>(text: &'a str, name: &str) -> &'a str {
    let close = format!("</{name}>");
    match text.strip_prefix(&format!("<{name}>")) {
        Some(rest) => rest
            .find(&close)
            .map_or(text, |end| &rest[end + close.len()..]),
        None => text,
    }
}

/// Returns the content of a single `<name>…</name>` pair that wraps the whole text.
fn strip_wrapping_pair<'a>(text: &'a str, name: &str) -> &'a str {
    let close = format!("</{name}>");
    text.strip_prefix(&format!("<{name}>"))
        .and_then(|rest| rest.strip_suffix(&close))
        .filter(|inner| !inner.contains(&close))
        .unwrap_or(text)
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

    const EDIT: EchoedTags = EchoedTags {
        unwrap: &["selection"],
        drop: &["instruction"],
    };

    #[test]
    fn tags_inside_the_content_are_kept() {
        for answer in [
            "Wrap it in <selection> tags.",
            "Use <selection>x</selection> here.",
            "Write <instruction>be brief</instruction> at the top.",
            "<selection>a</selection> and <selection>b</selection>",
        ] {
            assert_eq!(super::clean_answer(answer, EDIT), answer);
        }
    }

    #[test]
    fn instruction_block_is_dropped_only_when_it_leads() {
        assert_eq!(
            super::clean_answer("Done. <instruction>x</instruction>", EDIT),
            "Done. <instruction>x</instruction>"
        );
        assert_eq!(
            super::clean_answer("<instruction>x</instruction> Done.", EDIT),
            "Done."
        );
        assert_eq!(
            super::clean_answer("<instruction>unterminated Done.", EDIT),
            "<instruction>unterminated Done."
        );
    }

    #[test]
    fn wrapping_pair_keeps_tags_inside_the_edited_text() {
        assert_eq!(
            super::clean_answer("<selection>Put <selection> in the docs.</selection>", EDIT),
            "Put <selection> in the docs."
        );
        assert_eq!(
            super::clean_answer("  <selection>We meet.</selection>\n", EDIT),
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
