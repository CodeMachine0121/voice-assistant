//! Post-processing of the assistant's answer.

/// Removes reasoning blocks, code fences and surrounding quotes, then trims.
pub(crate) fn clean_answer(answer: &str) -> String {
    let text = strip_think(answer);
    let text = strip_fence(text.trim());
    strip_quotes(text.trim()).trim().to_string()
}

fn strip_think(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</think>") {
            Some(end) => rest = &rest[start + end + "</think>".len()..],
            // Unterminated block: everything after it is reasoning.
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
    use super::clean_answer;

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
}
