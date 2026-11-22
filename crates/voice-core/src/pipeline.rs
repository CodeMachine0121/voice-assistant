//! Finishing a dictation: speech check → transcribe → polish (or skip) → insert → history.
use crate::speech::contains_speech;
use crate::{
    History, HistoryEntry, HistoryStore, PersonalDictionary, PolishError, Polisher, Problem,
    Recording, Settings, Status, TextInserter, Transcriber,
};
use chrono::Local;
use std::sync::{Arc, Mutex, PoisonError};

/// Result of processing one recording.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// Final status to show (Done, or Problem(..)).
    pub status: Status,
    /// The history entry recorded, if any.
    pub entry: Option<HistoryEntry>,
}

impl Outcome {
    fn problem_without_entry(problem: Problem) -> Self {
        Self {
            status: Status::Problem(problem),
            entry: None,
        }
    }
}

pub struct DictationPipeline {
    transcriber: Arc<dyn Transcriber>,
    polisher: Arc<dyn Polisher>,
    inserter: Arc<dyn TextInserter>,
    store: Arc<dyn HistoryStore>,
    history: Mutex<History>,
}

impl DictationPipeline {
    pub fn new(
        transcriber: Arc<dyn Transcriber>,
        polisher: Arc<dyn Polisher>,
        inserter: Arc<dyn TextInserter>,
        history: Arc<dyn HistoryStore>,
    ) -> Self {
        let loaded = history.load();
        Self {
            transcriber,
            polisher,
            inserter,
            store: history,
            history: Mutex::new(loaded),
        }
    }

    pub fn process(&self, recording: Recording, settings: &Settings) -> Outcome {
        if !contains_speech(&recording) {
            return Outcome::problem_without_entry(Problem::NoSpeech);
        }
        let raw = match self
            .transcriber
            .transcribe(&recording, &settings.dictionary)
        {
            Ok(raw) => raw.trim().to_string(),
            Err(e) => return Outcome::problem_without_entry(Problem::Other(e)),
        };
        if raw.is_empty() {
            return Outcome::problem_without_entry(Problem::NoSpeech);
        }

        let (text, assistant_problem) = if settings.cleanup_enabled {
            self.polish(&raw, &settings.dictionary)
        } else {
            (raw.clone(), None)
        };

        // An accessibility problem matters more than skipped cleanup: the text did not land.
        let problem = self.inserter.insert(&text).err().or(assistant_problem);

        let entry = HistoryEntry {
            at: Local::now(),
            raw,
            polished: text,
        };
        self.record(entry.clone());

        Outcome {
            status: problem.map_or(Status::Done, Status::Problem),
            entry: Some(entry),
        }
    }

    /// Polished text, or the raw transcript when cleanup could not be done.
    fn polish(&self, raw: &str, dictionary: &PersonalDictionary) -> (String, Option<Problem>) {
        match self.polisher.polish(raw, dictionary) {
            Ok(polished) if !polished.trim().is_empty() => (polished.trim().to_string(), None),
            Ok(_) => (raw.to_string(), None),
            Err(PolishError::Unreachable(reason)) => {
                log::warn!("writing assistant unreachable: {reason}");
                (raw.to_string(), Some(Problem::AssistantUnreachable))
            }
            Err(PolishError::InvalidResponse(reason)) => {
                log::warn!("writing assistant gave an unusable answer: {reason}");
                (raw.to_string(), None)
            }
        }
    }

    fn record(&self, entry: HistoryEntry) {
        let mut history = self.history.lock().unwrap_or_else(PoisonError::into_inner);
        history.record(entry);
        if let Err(e) = self.store.save(&history) {
            log::warn!("could not save history: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{silence, speech, FakeInserter, FakePolisher, FakeStore, FakeTranscriber};
    use crate::rules::HISTORY_LIMIT;
    use chrono::TimeZone;

    struct Rig {
        transcriber: Arc<FakeTranscriber>,
        polisher: Arc<FakePolisher>,
        inserter: Arc<FakeInserter>,
        store: Arc<FakeStore>,
    }

    impl Rig {
        fn new(raw: &str, polished: Result<&str, PolishError>) -> Self {
            Self {
                transcriber: FakeTranscriber::new(Ok(raw)),
                polisher: FakePolisher::new(polished),
                inserter: FakeInserter::new(Ok(())),
                store: FakeStore::new(History::default()),
            }
        }
        fn pipeline(&self) -> DictationPipeline {
            DictationPipeline::new(
                self.transcriber.clone(),
                self.polisher.clone(),
                self.inserter.clone(),
                self.store.clone(),
            )
        }
        fn last_saved(&self) -> History {
            self.store.saved().pop().expect("history saved")
        }
    }

    fn cleanup_on() -> Settings {
        Settings::default()
    }

    fn cleanup_off() -> Settings {
        Settings {
            cleanup_enabled: false,
            ..Settings::default()
        }
    }

    fn old_entry(n: usize) -> HistoryEntry {
        HistoryEntry {
            at: Local.timestamp_opt(1_600_000_000 + n as i64, 0).unwrap(),
            raw: format!("old {n}"),
            polished: format!("Old {n}."),
        }
    }

    // ---- happy path --------------------------------------------------------

    #[test]
    fn polished_text_is_inserted_recorded_and_done() {
        let rig = Rig::new("hello team", Ok("Hello team."));

        let outcome = rig.pipeline().process(speech(), &cleanup_on());

        assert_eq!(outcome.status, Status::Done);
        assert_eq!(rig.inserter.inserted(), ["Hello team."]);
        let entry = outcome.entry.expect("entry");
        assert_eq!(entry.raw, "hello team");
        assert_eq!(entry.polished, "Hello team.");
        assert_eq!(rig.last_saved().entries(), [entry]);
    }

    #[test]
    fn transcript_and_polished_text_are_trimmed() {
        let rig = Rig::new("  hello team \n", Ok("\n Hello team. "));

        let outcome = rig.pipeline().process(speech(), &cleanup_on());

        assert_eq!(rig.polisher.calls()[0].0, "hello team");
        assert_eq!(rig.inserter.inserted(), ["Hello team."]);
        assert_eq!(outcome.entry.map(|e| e.raw), Some("hello team".into()));
    }

    #[test]
    fn dictionary_reaches_transcriber_and_polisher() {
        let rig = Rig::new("I work at caffler", Ok("I work at Cafler."));
        let settings = Settings {
            dictionary: PersonalDictionary::from_words(["Cafler"]),
            ..Settings::default()
        };

        rig.pipeline().process(speech(), &settings);

        assert_eq!(rig.transcriber.dictionaries()[0].words(), ["Cafler"]);
        assert_eq!(rig.polisher.calls()[0].1.words(), ["Cafler"]);
    }

    // ---- cleanup off -------------------------------------------------------

    #[test]
    fn cleanup_off_writes_the_raw_transcript_unchanged() {
        let rig = Rig::new("um hello there", Ok("Hello there."));

        let outcome = rig.pipeline().process(speech(), &cleanup_off());

        assert_eq!(rig.inserter.inserted(), ["um hello there"]);
        assert!(rig.polisher.calls().is_empty());
        assert_eq!(outcome.status, Status::Done);
        assert_eq!(
            outcome.entry.map(|e| e.polished),
            Some("um hello there".into())
        );
    }

    // ---- fallbacks ---------------------------------------------------------

    #[test]
    fn unreachable_assistant_writes_raw_and_reports_it() {
        let rig = Rig::new(
            "hello there",
            Err(PolishError::Unreachable("connection refused".into())),
        );

        let outcome = rig.pipeline().process(speech(), &cleanup_on());

        assert_eq!(rig.inserter.inserted(), ["hello there"]);
        assert_eq!(
            outcome.status,
            Status::Problem(Problem::AssistantUnreachable)
        );
        assert_eq!(
            Problem::AssistantUnreachable.to_string(),
            "Cleanup skipped – assistant unreachable"
        );
        assert_eq!(
            outcome.entry.map(|e| e.polished),
            Some("hello there".into())
        );
    }

    #[test]
    fn slow_assistant_reported_as_unreachable_writes_raw() {
        let rig = Rig::new(
            "hello there",
            Err(PolishError::Unreachable("timed out after 20 s".into())),
        );

        let outcome = rig.pipeline().process(speech(), &cleanup_on());

        assert_eq!(rig.inserter.inserted(), ["hello there"]);
        assert_eq!(
            outcome.status,
            Status::Problem(Problem::AssistantUnreachable)
        );
    }

    #[test]
    fn empty_assistant_answer_falls_back_to_raw_silently() {
        let rig = Rig::new("hello there", Ok("   "));

        let outcome = rig.pipeline().process(speech(), &cleanup_on());

        assert_eq!(rig.inserter.inserted(), ["hello there"]);
        assert_eq!(outcome.status, Status::Done);
    }

    #[test]
    fn invalid_assistant_answer_falls_back_to_raw_silently() {
        let rig = Rig::new(
            "hello there",
            Err(PolishError::InvalidResponse("no choices".into())),
        );

        let outcome = rig.pipeline().process(speech(), &cleanup_on());

        assert_eq!(rig.inserter.inserted(), ["hello there"]);
        assert_eq!(outcome.status, Status::Done);
        assert_eq!(
            outcome.entry.map(|e| e.polished),
            Some("hello there".into())
        );
    }

    // ---- silence -----------------------------------------------------------

    #[test]
    fn silence_writes_nothing_and_records_nothing() {
        let rig = Rig::new("Thank you.", Ok("Thank you."));

        let outcome = rig.pipeline().process(silence(), &cleanup_on());

        assert_eq!(outcome.status, Status::Problem(Problem::NoSpeech));
        assert_eq!(outcome.entry, None);
        assert_eq!(rig.transcriber.calls(), 0);
        assert!(rig.polisher.calls().is_empty());
        assert!(rig.inserter.inserted().is_empty());
        assert!(rig.store.saved().is_empty());
    }

    #[test]
    fn blank_transcript_counts_as_no_speech() {
        let rig = Rig::new("  \n ", Ok("Hello."));

        let outcome = rig.pipeline().process(speech(), &cleanup_on());

        assert_eq!(outcome.status, Status::Problem(Problem::NoSpeech));
        assert_eq!(outcome.entry, None);
        assert!(rig.polisher.calls().is_empty());
        assert!(rig.inserter.inserted().is_empty());
        assert!(rig.store.saved().is_empty());
    }

    #[test]
    fn transcription_failure_is_reported_and_nothing_written() {
        let rig = Rig {
            transcriber: FakeTranscriber::new(Err("model missing")),
            ..Rig::new("", Ok("x"))
        };

        let outcome = rig.pipeline().process(speech(), &cleanup_on());

        assert_eq!(
            outcome.status,
            Status::Problem(Problem::Other("model missing".into()))
        );
        assert_eq!(outcome.entry, None);
        assert!(rig.inserter.inserted().is_empty());
        assert!(rig.store.saved().is_empty());
    }

    // ---- accessibility -----------------------------------------------------

    #[test]
    fn missing_accessibility_still_records_history() {
        let rig = Rig {
            inserter: FakeInserter::new(Err(Problem::AccessibilityPermission)),
            ..Rig::new("hello", Ok("Hello."))
        };

        let outcome = rig.pipeline().process(speech(), &cleanup_on());

        assert_eq!(
            outcome.status,
            Status::Problem(Problem::AccessibilityPermission)
        );
        assert_eq!(rig.inserter.inserted(), ["Hello."]);
        assert_eq!(rig.last_saved().entries()[0].polished, "Hello.");
        assert_eq!(outcome.entry.map(|e| e.polished), Some("Hello.".into()));
    }

    #[test]
    fn accessibility_problem_wins_over_unreachable_assistant() {
        let rig = Rig {
            inserter: FakeInserter::new(Err(Problem::AccessibilityPermission)),
            ..Rig::new("hello", Err(PolishError::Unreachable("down".into())))
        };

        let outcome = rig.pipeline().process(speech(), &cleanup_on());

        assert_eq!(
            outcome.status,
            Status::Problem(Problem::AccessibilityPermission)
        );
    }

    // ---- history -----------------------------------------------------------

    #[test]
    fn history_is_loaded_once_and_new_entries_go_first() {
        let rig = Rig {
            store: FakeStore::new(History::from(vec![old_entry(1)])),
            ..Rig::new("hello", Ok("Hello."))
        };
        let pipeline = rig.pipeline();

        pipeline.process(speech(), &cleanup_on());
        pipeline.process(speech(), &cleanup_on());

        let saved = rig.last_saved();
        let polished: Vec<_> = saved
            .entries()
            .iter()
            .map(|e| e.polished.as_str())
            .collect();
        assert_eq!(polished, ["Hello.", "Hello.", "Old 1."]);
        assert!(saved.entries()[0].at >= saved.entries()[1].at);
    }

    #[test]
    fn full_history_drops_the_oldest_when_a_dictation_finishes() {
        let full: Vec<_> = (1..=HISTORY_LIMIT).rev().map(old_entry).collect();
        let rig = Rig {
            store: FakeStore::new(History::from(full)),
            ..Rig::new("hello", Ok("Hello."))
        };

        rig.pipeline().process(speech(), &cleanup_on());

        let saved = rig.last_saved();
        assert_eq!(saved.entries().len(), HISTORY_LIMIT);
        assert_eq!(saved.entries()[0].polished, "Hello.");
        assert!(!saved.entries().contains(&old_entry(1)));
    }

    #[test]
    fn history_save_failure_does_not_change_the_outcome() {
        let rig = Rig {
            store: FakeStore::failing(),
            ..Rig::new("hello", Ok("Hello."))
        };
        let pipeline = rig.pipeline();

        let first = pipeline.process(speech(), &cleanup_on());
        pipeline.process(speech(), &cleanup_on());

        assert_eq!(first.status, Status::Done);
        assert!(first.entry.is_some());
        // the in-memory history keeps growing even though saving fails
        assert_eq!(rig.last_saved().entries().len(), 2);
    }
}
