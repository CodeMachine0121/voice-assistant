use serde::{Deserialize, Serialize};

/// Words that must always be spelled exactly as entered.
/// Invariant: entries are trimmed, non-blank, unique (case-sensitive), insertion-ordered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Vec<String>", into = "Vec<String>")]
pub struct PersonalDictionary {
    words: Vec<String>,
}

impl PersonalDictionary {
    pub fn from_words<I, S>(words: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut dictionary = Self::default();
        for word in words {
            dictionary.add(&word.into());
        }
        dictionary
    }

    /// Adds a word; blanks and duplicates are ignored. Returns whether it was added.
    pub fn add(&mut self, word: &str) -> bool {
        let word = word.trim();
        if word.is_empty() || self.words.iter().any(|w| w == word) {
            return false;
        }
        self.words.push(word.to_string());
        true
    }

    /// Removes a word (compared after trimming, case-sensitive); unknown words are ignored.
    pub fn remove(&mut self, word: &str) {
        let word = word.trim();
        self.words.retain(|w| w != word);
    }

    pub fn words(&self) -> &[String] {
        &self.words
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

impl From<Vec<String>> for PersonalDictionary {
    fn from(words: Vec<String>) -> Self {
        Self::from_words(words)
    }
}

impl From<PersonalDictionary> for Vec<String> {
    fn from(d: PersonalDictionary) -> Self {
        d.words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cafler() -> PersonalDictionary {
        PersonalDictionary::from_words(["Cafler"])
    }

    #[test]
    fn adding_an_existing_word_keeps_it_once() {
        let mut dictionary = cafler();

        let added = dictionary.add("Cafler");

        assert!(!added);
        assert_eq!(dictionary.words(), ["Cafler"]);
    }

    #[test]
    fn adding_a_blank_entry_is_ignored() {
        let mut dictionary = cafler();

        let added = dictionary.add("   ");

        assert!(!added);
        assert_eq!(dictionary.words(), ["Cafler"]);
    }

    #[test]
    fn adding_a_new_word_stores_it_trimmed_after_existing_ones() {
        let mut dictionary = cafler();

        let added = dictionary.add("  Typeless ");

        assert!(added);
        assert_eq!(dictionary.words(), ["Cafler", "Typeless"]);
    }

    #[test]
    fn duplicates_are_case_sensitive() {
        let mut dictionary = cafler();

        assert!(dictionary.add("cafler"));
        assert_eq!(dictionary.words(), ["Cafler", "cafler"]);
    }

    #[test]
    fn a_padded_duplicate_is_still_a_duplicate() {
        let mut dictionary = cafler();

        assert!(!dictionary.add(" Cafler\t"));
        assert_eq!(dictionary.words(), ["Cafler"]);
    }

    #[test]
    fn from_words_trims_drops_blanks_and_duplicates_keeping_order() {
        let dictionary =
            PersonalDictionary::from_words(["  Zed", "", "Cafler", "Zed", "  ", "Alpha"]);

        assert_eq!(dictionary.words(), ["Zed", "Cafler", "Alpha"]);
    }

    #[test]
    fn remove_deletes_only_the_matching_word() {
        let mut dictionary = PersonalDictionary::from_words(["Cafler", "Typeless"]);

        dictionary.remove("Cafler");

        assert_eq!(dictionary.words(), ["Typeless"]);
    }

    #[test]
    fn remove_matches_trimmed_input_and_ignores_unknown_words() {
        let mut dictionary = PersonalDictionary::from_words(["Cafler", "Typeless"]);

        dictionary.remove(" Typeless ");
        dictionary.remove("Unknown");

        assert_eq!(dictionary.words(), ["Cafler"]);
    }

    #[test]
    fn is_empty_reflects_contents() {
        assert!(PersonalDictionary::default().is_empty());
        assert!(!cafler().is_empty());
        assert!(PersonalDictionary::from_words(["", " "]).is_empty());
    }

    #[test]
    fn serializes_as_a_json_array_of_strings() {
        let dictionary = PersonalDictionary::from_words(["Cafler", "Typeless"]);

        let json = serde_json::to_string(&dictionary).unwrap();

        assert_eq!(json, r#"["Cafler","Typeless"]"#);
    }

    #[test]
    fn deserializing_enforces_the_invariant() {
        let dictionary: PersonalDictionary =
            serde_json::from_str(r#"["Cafler", " Cafler ", "", "Typeless"]"#).unwrap();

        assert_eq!(dictionary.words(), ["Cafler", "Typeless"]);
    }

    #[test]
    fn from_vec_enforces_the_invariant() {
        let dictionary = PersonalDictionary::from(vec![" a ".to_string(), "a".to_string()]);

        assert_eq!(Vec::<String>::from(dictionary), vec!["a".to_string()]);
    }
}
