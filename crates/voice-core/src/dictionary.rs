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
        todo!("enforce invariant")
    }

    /// Adds a word; blanks and duplicates are ignored. Returns whether it was added.
    pub fn add(&mut self, word: &str) -> bool {
        todo!()
    }

    pub fn remove(&mut self, word: &str) {
        todo!()
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
