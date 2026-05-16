/// Returns true when `query` looks like a natural-language question.
///
/// Triggers on: trailing `?`, interrogative prefix, or more than 5 words.
pub fn is_question(query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return false;
    }
    if q.ends_with('?') {
        return true;
    }
    let words: Vec<&str> = q.split_whitespace().collect();
    if words.len() > 5 {
        return true;
    }
    let first = words.first().map(|w| w.to_ascii_lowercase()).unwrap_or_default();
    matches!(
        first.as_str(),
        "qui" | "quoi" | "comment" | "pourquoi" | "quand" | "où" | "quel" | "quelle"
            | "why" | "how" | "what" | "when" | "where" | "who" | "which"
            | "is" | "are" | "can" | "does" | "do" | "will" | "would" | "should"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn question_mark() {
        assert!(is_question("comment trier un Vec?"));
    }

    #[test]
    fn interrogative_prefix() {
        assert!(is_question("how do I reverse a string in Rust"));
    }

    #[test]
    fn long_query() {
        assert!(is_question("the quick brown fox jumps over"));
    }

    #[test]
    fn short_filename() {
        assert!(!is_question("main.rs"));
    }
}
