//! Finishing a recording: speech check → heard words → writing assistant (per action)
//! → insert at the cursor → history.
use crate::script::to_traditional;
use crate::speech::contains_speech;
use crate::{
    Action, History, HistoryEntry, HistoryStore, PersonalDictionary, PolishError, Polisher,
    PreparedAction, Problem, Recording, Settings, Status, TextInserter, TranscribeError,
    Transcriber, WritingTask,
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

/// What the writing assistant gave back.
enum Reply {
    Text(String),
    /// Answered, but with nothing usable.
    Unusable,
    Unreachable,
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

    pub fn process(
        &self,
        recording: Recording,
        action: &PreparedAction,
        settings: &Settings,
    ) -> Outcome {
        if !contains_speech(&recording) {
            return Outcome::problem_without_entry(Problem::NoSpeech);
        }
        let heard = match self
            .transcriber
            .transcribe(&recording, &settings.dictionary)
        {
            Ok(transcript) => to_traditional(transcript.trim()),
            Err(TranscribeError::Unreachable(reason)) => {
                log::warn!("speech model unreachable: {reason}");
                return Outcome::problem_without_entry(Problem::SpeechModelUnreachable);
            }
            Err(TranscribeError::Failed(reason)) => {
                return Outcome::problem_without_entry(Problem::Other(reason))
            }
        };
        if heard.is_empty() {
            return Outcome::problem_without_entry(Problem::NoSpeech);
        }

        let reply = action
            .writing_task(&heard, settings.cleanup_enabled)
            .map(|task| self.ask(&task, &settings.dictionary));
        let (text, assistant_problem) = match (action, reply) {
            (_, None) => (heard.clone(), None),
            // The selection stays as it was: nothing to write, nothing to record.
            (PreparedAction::EditSelection { .. }, Some(Reply::Unusable | Reply::Unreachable)) => {
                return Outcome::problem_without_entry(Problem::AssistantUnreachable(
                    Action::EditSelection,
                ))
            }
            // Only Dictate keeps the speaker's language, so only it is forced to Traditional;
            // an Edit instruction may ask for Simplified.
            (PreparedAction::Dictate, Some(Reply::Text(result))) => (to_traditional(&result), None),
            (_, Some(Reply::Text(result))) => (result, None),
            // Writing at the cursor falls back to the heard words.
            (_, Some(Reply::Unusable)) => (heard.clone(), None),
            (_, Some(Reply::Unreachable)) => (
                heard.clone(),
                Some(Problem::AssistantUnreachable(action.action())),
            ),
        };
        let original = match action {
            PreparedAction::EditSelection { selection } => Some(selection.clone()),
            _ => None,
        };

        // An accessibility problem matters more than skipped cleanup: the text did not land.
        let problem = self.inserter.insert(&text).err().or(assistant_problem);

        let entry = HistoryEntry {
            at: Local::now(),
            action: action.action(),
            raw: heard,
            original,
            polished: text,
        };
        self.record(entry.clone());

        Outcome {
            status: problem.map_or(Status::Done, Status::Problem),
            entry: Some(entry),
        }
    }

    fn ask(&self, task: &WritingTask<'_>, dictionary: &PersonalDictionary) -> Reply {
        match self.polisher.polish(task, dictionary) {
            Ok(text) if !text.trim().is_empty() => Reply::Text(text.trim().to_string()),
            Ok(_) => {
                log::warn!("writing assistant gave an empty answer");
                Reply::Unusable
            }
            Err(PolishError::Unreachable(reason)) => {
                log::warn!("writing assistant unreachable: {reason}");
                Reply::Unreachable
            }
            Err(PolishError::InvalidResponse(reason)) => {
                log::warn!("writing assistant gave an unusable answer: {reason}");
                Reply::Unusable
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
    use crate::fakes::{
        silence, speech, FakeInserter, FakePolisher, FakeStore, FakeTranscriber, Task,
    };
    use crate::rules::HISTORY_LIMIT;
    use chrono::TimeZone;

    struct Rig {
        transcriber: Arc<FakeTranscriber>,
        polisher: Arc<FakePolisher>,
        inserter: Arc<FakeInserter>,
        store: Arc<FakeStore>,
    }

    impl Rig {
        fn new(heard: &str, answer: Result<&str, PolishError>) -> Self {
            Self {
                transcriber: FakeTranscriber::new(Ok(heard)),
                polisher: FakePolisher::new(answer),
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
        fn run(&self, action: &PreparedAction, settings: &Settings) -> Outcome {
            self.pipeline().process(speech(), action, settings)
        }
        fn last_saved(&self) -> History {
            self.store.saved().pop().expect("history saved")
        }
        fn assert_nothing_written_or_recorded(&self) {
            assert!(self.inserter.inserted().is_empty(), "nothing written");
            assert!(self.store.saved().is_empty(), "nothing recorded");
        }
    }

    fn unreachable() -> Result<&'static str, PolishError> {
        Err(PolishError::Unreachable("connection refused".into()))
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

    fn edit(selection: &str) -> PreparedAction {
        PreparedAction::EditSelection {
            selection: selection.into(),
        }
    }

    const DICTATE: PreparedAction = PreparedAction::Dictate;
    const ENGLISH: PreparedAction = PreparedAction::DictateInEnglish;

    fn old_entry(n: usize) -> HistoryEntry {
        HistoryEntry {
            at: Local.timestamp_opt(1_600_000_000 + n as i64, 0).unwrap(),
            action: Action::Dictate,
            raw: format!("old {n}"),
            original: None,
            polished: format!("Old {n}."),
        }
    }

    // ---- Dictate -----------------------------------------------------------

    #[test]
    fn dictate_cleans_up_inserts_records_and_is_done() {
        let rig = Rig::new("我們明天下午三點開會", Ok("我們明天下午三點開會。"));

        let outcome = rig.run(&DICTATE, &cleanup_on());

        assert_eq!(outcome.status, Status::Done);
        assert_eq!(
            rig.polisher.tasks(),
            [Task::CleanUp("我們明天下午三點開會".into())]
        );
        assert_eq!(rig.inserter.inserted(), ["我們明天下午三點開會。"]);
        let entry = outcome.entry.expect("entry");
        assert_eq!(entry.action, Action::Dictate);
        assert_eq!(entry.raw, "我們明天下午三點開會");
        assert_eq!(entry.original, None);
        assert_eq!(entry.polished, "我們明天下午三點開會。");
        assert_eq!(rig.last_saved().entries(), [entry]);
    }

    #[test]
    fn dictate_writes_simplified_heard_words_in_traditional_characters() {
        let rig = Rig::new("我们开会", Ok("我们开会。"));

        let outcome = rig.run(&DICTATE, &cleanup_on());

        assert_eq!(rig.polisher.tasks(), [Task::CleanUp("我們開會".into())]);
        assert_eq!(rig.inserter.inserted(), ["我們開會。"]);
        assert_eq!(outcome.entry.map(|e| e.raw), Some("我們開會".into()));
    }

    #[test]
    fn dictate_keeps_mixed_chinese_and_english() {
        let rig = Rig::new("嗯 我們那個 review 一下 PR", Ok("我們 review 一下 PR。"));

        rig.run(&DICTATE, &cleanup_on());

        assert_eq!(rig.inserter.inserted(), ["我們 review 一下 PR。"]);
    }

    #[test]
    fn cleanup_off_writes_the_heard_words_unchanged() {
        let rig = Rig::new("嗯 我們開會", Ok("我們開會。"));

        let outcome = rig.run(&DICTATE, &cleanup_off());

        assert_eq!(rig.inserter.inserted(), ["嗯 我們開會"]);
        assert!(rig.polisher.calls().is_empty());
        assert_eq!(outcome.status, Status::Done);
        assert_eq!(
            outcome.entry.map(|e| e.polished),
            Some("嗯 我們開會".into())
        );
    }

    #[test]
    fn cleanup_off_still_writes_chinese_in_traditional_characters() {
        let rig = Rig::new("我们开会", Ok("x"));

        rig.run(&DICTATE, &cleanup_off());

        assert_eq!(rig.inserter.inserted(), ["我們開會"]);
    }

    #[test]
    fn unreachable_assistant_during_dictate_writes_the_heard_words_and_says_so() {
        let rig = Rig::new("我們開會", unreachable());

        let outcome = rig.run(&DICTATE, &cleanup_on());

        assert_eq!(rig.inserter.inserted(), ["我們開會"]);
        assert_eq!(
            outcome.status,
            Status::Problem(Problem::AssistantUnreachable(Action::Dictate))
        );
        assert_eq!(
            outcome.status_message(),
            "AI cleanup skipped — writing assistant not reachable"
        );
        assert_eq!(outcome.entry.map(|e| e.polished), Some("我們開會".into()));
    }

    #[test]
    fn empty_answer_during_dictate_falls_back_to_heard_words_silently() {
        let rig = Rig::new("hello there", Ok("   "));

        let outcome = rig.run(&DICTATE, &cleanup_on());

        assert_eq!(rig.inserter.inserted(), ["hello there"]);
        assert_eq!(outcome.status, Status::Done);
    }

    #[test]
    fn invalid_answer_during_dictate_falls_back_to_heard_words_silently() {
        let rig = Rig::new(
            "hello there",
            Err(PolishError::InvalidResponse("no choices".into())),
        );

        let outcome = rig.run(&DICTATE, &cleanup_on());

        assert_eq!(rig.inserter.inserted(), ["hello there"]);
        assert_eq!(outcome.status, Status::Done);
        assert_eq!(
            outcome.entry.map(|e| e.polished),
            Some("hello there".into())
        );
    }

    // ---- Dictate in English ------------------------------------------------

    #[test]
    fn english_translates_inserts_and_records_an_english_entry() {
        let rig = Rig::new("我們開會", Ok("We have a meeting."));

        let outcome = rig.run(&ENGLISH, &cleanup_on());

        assert_eq!(outcome.status, Status::Done);
        assert_eq!(
            rig.polisher.tasks(),
            [Task::TranslateToEnglish("我們開會".into())]
        );
        assert_eq!(rig.inserter.inserted(), ["We have a meeting."]);
        let entry = rig.last_saved().entries()[0].clone();
        assert_eq!(entry.action, Action::DictateInEnglish);
        assert_eq!(entry.action.label(), "English");
        assert_eq!(entry.raw, "我們開會");
        assert_eq!(entry.polished, "We have a meeting.");
        assert_eq!(entry.original, None);
    }

    #[test]
    fn english_still_translates_when_cleanup_is_off() {
        let rig = Rig::new("我們開會", Ok("We have a meeting."));

        rig.run(&ENGLISH, &cleanup_off());

        assert_eq!(
            rig.polisher.tasks(),
            [Task::TranslateToEnglish("我們開會".into())]
        );
        assert_eq!(rig.inserter.inserted(), ["We have a meeting."]);
    }

    #[test]
    fn english_sends_simplified_heard_words_as_traditional() {
        let rig = Rig::new("这个 bug 我下星期修", Ok("I'll fix this bug next week."));

        rig.run(&ENGLISH, &cleanup_on());

        assert_eq!(
            rig.polisher.tasks(),
            [Task::TranslateToEnglish("這個 bug 我下星期修".into())]
        );
        assert_eq!(rig.inserter.inserted(), ["I'll fix this bug next week."]);
    }

    #[test]
    fn unreachable_assistant_during_english_writes_the_heard_words_and_says_so() {
        let rig = Rig::new("我們開會", unreachable());

        let outcome = rig.run(&ENGLISH, &cleanup_on());

        assert_eq!(rig.inserter.inserted(), ["我們開會"]);
        assert_eq!(
            outcome.status,
            Status::Problem(Problem::AssistantUnreachable(Action::DictateInEnglish))
        );
        assert_eq!(
            outcome.status_message(),
            "Translation skipped — writing assistant not reachable"
        );
        assert_eq!(
            outcome.entry.map(|e| e.action),
            Some(Action::DictateInEnglish)
        );
    }

    #[test]
    fn unusable_answer_during_english_falls_back_to_heard_words_silently() {
        let rig = Rig::new("我們開會", Err(PolishError::InvalidResponse("bad".into())));

        let outcome = rig.run(&ENGLISH, &cleanup_on());

        assert_eq!(rig.inserter.inserted(), ["我們開會"]);
        assert_eq!(outcome.status, Status::Done);
    }

    // ---- Edit selection ----------------------------------------------------

    #[test]
    fn edit_applies_the_instruction_to_the_selection_and_replaces_it() {
        let rig = Rig::new("translate to English", Ok("We have a meeting tomorrow."));

        let outcome = rig.run(&edit("我們明天開會"), &cleanup_on());

        assert_eq!(outcome.status, Status::Done);
        assert_eq!(
            rig.polisher.tasks(),
            [Task::EditSelection {
                instruction: "translate to English".into(),
                selection: "我們明天開會".into(),
            }]
        );
        // inserting while the text is still selected replaces it
        assert_eq!(rig.inserter.inserted(), ["We have a meeting tomorrow."]);
    }

    #[test]
    fn edit_records_instruction_original_and_result() {
        let rig = Rig::new("translate to English", Ok("We have a meeting tomorrow."));

        rig.run(&edit("我們明天開會"), &cleanup_on());

        let entry = rig.last_saved().entries()[0].clone();
        assert_eq!(entry.action, Action::EditSelection);
        assert_eq!(entry.action.label(), "Edit");
        assert_eq!(entry.raw, "translate to English");
        assert_eq!(entry.original.as_deref(), Some("我們明天開會"));
        assert_eq!(entry.polished, "We have a meeting tomorrow.");
    }

    #[test]
    fn edit_runs_even_when_cleanup_is_off() {
        let rig = Rig::new("幫我翻譯成中文", Ok("你好，你好嗎？"));

        rig.run(&edit("Hello, how are you?"), &cleanup_off());

        assert_eq!(rig.inserter.inserted(), ["你好，你好嗎？"]);
    }

    #[test]
    fn edit_writes_the_result_as_given_so_an_instruction_may_ask_for_simplified() {
        let rig = Rig::new("翻譯成簡體中文", Ok("我们明天开会"));

        rig.run(&edit("我們明天開會"), &cleanup_on());

        assert_eq!(rig.inserter.inserted(), ["我们明天开会"]);
    }

    #[test]
    fn edit_receives_a_simplified_instruction_as_traditional() {
        let rig = Rig::new("缩短成一句话", Ok("One sentence."));

        rig.run(&edit("First. Second. Third."), &cleanup_on());

        assert_eq!(
            rig.polisher.tasks(),
            [Task::EditSelection {
                instruction: "縮短成一句話".into(),
                selection: "First. Second. Third.".into(),
            }]
        );
    }

    #[test]
    fn unreachable_assistant_during_edit_changes_nothing_and_records_nothing() {
        let rig = Rig::new("translate to English", unreachable());

        let outcome = rig.run(&edit("我們明天開會"), &cleanup_on());

        assert_eq!(
            outcome.status,
            Status::Problem(Problem::AssistantUnreachable(Action::EditSelection))
        );
        assert_eq!(
            outcome.status_message(),
            "Couldn't run the instruction — writing assistant not reachable"
        );
        assert_eq!(outcome.entry, None);
        rig.assert_nothing_written_or_recorded();
    }

    #[test]
    fn unusable_answer_during_edit_changes_nothing_and_says_so() {
        for answer in [Ok("  \n"), Err(PolishError::InvalidResponse("bad".into()))] {
            let rig = Rig::new("translate to English", answer);

            let outcome = rig.run(&edit("我們明天開會"), &cleanup_on());

            assert_eq!(
                outcome.status,
                Status::Problem(Problem::AssistantUnreachable(Action::EditSelection))
            );
            assert_eq!(outcome.entry, None);
            rig.assert_nothing_written_or_recorded();
        }
    }

    #[test]
    fn silence_during_edit_changes_nothing() {
        let rig = Rig::new("Thank you.", Ok("Thank you."));

        let outcome = rig
            .pipeline()
            .process(silence(), &edit("我們明天開會"), &cleanup_on());

        assert_eq!(outcome.status, Status::Problem(Problem::NoSpeech));
        assert_eq!(outcome.status_message(), "No speech heard");
        assert!(rig.polisher.calls().is_empty());
        rig.assert_nothing_written_or_recorded();
    }

    // ---- shared rules ------------------------------------------------------

    #[test]
    fn heard_words_and_answer_are_trimmed() {
        let rig = Rig::new("  hello team \n", Ok("\n Hello team. "));

        let outcome = rig.run(&DICTATE, &cleanup_on());

        assert_eq!(rig.polisher.tasks(), [Task::CleanUp("hello team".into())]);
        assert_eq!(rig.inserter.inserted(), ["Hello team."]);
        assert_eq!(outcome.entry.map(|e| e.raw), Some("hello team".into()));
    }

    #[test]
    fn dictionary_reaches_speech_model_and_writing_assistant_in_every_action() {
        let settings = Settings {
            dictionary: PersonalDictionary::from_words(["Cafler"]),
            ..Settings::default()
        };
        for action in [DICTATE, ENGLISH, edit("x")] {
            let rig = Rig::new("I work at caffler", Ok("I work at Cafler."));

            rig.run(&action, &settings);

            assert_eq!(rig.transcriber.dictionaries()[0].words(), ["Cafler"]);
            assert_eq!(rig.polisher.calls()[0].1.words(), ["Cafler"]);
        }
    }

    #[test]
    fn silence_writes_nothing_and_records_nothing_in_any_action() {
        for action in [DICTATE, ENGLISH, edit("x")] {
            let rig = Rig::new("Thank you.", Ok("Thank you."));

            let outcome = rig.pipeline().process(silence(), &action, &cleanup_on());

            assert_eq!(outcome.status, Status::Problem(Problem::NoSpeech));
            assert_eq!(outcome.entry, None);
            assert_eq!(rig.transcriber.calls(), 0);
            assert!(rig.polisher.calls().is_empty());
            rig.assert_nothing_written_or_recorded();
        }
    }

    #[test]
    fn blank_heard_words_count_as_no_speech() {
        let rig = Rig::new("  \n ", Ok("Hello."));

        let outcome = rig.run(&DICTATE, &cleanup_on());

        assert_eq!(outcome.status, Status::Problem(Problem::NoSpeech));
        assert_eq!(outcome.entry, None);
        assert!(rig.polisher.calls().is_empty());
        rig.assert_nothing_written_or_recorded();
    }

    #[test]
    fn unreachable_speech_model_writes_nothing_in_any_action() {
        for action in [DICTATE, ENGLISH, edit("我們明天開會")] {
            let rig = Rig {
                transcriber: FakeTranscriber::new(Err(TranscribeError::Unreachable(
                    "connection refused".into(),
                ))),
                ..Rig::new("", Ok("x"))
            };

            let outcome = rig.run(&action, &cleanup_on());

            assert_eq!(
                outcome.status,
                Status::Problem(Problem::SpeechModelUnreachable)
            );
            assert_eq!(
                outcome.status_message(),
                "Speech model not reachable — is your local AI app running?"
            );
            assert_eq!(outcome.entry, None);
            assert!(rig.polisher.calls().is_empty());
            rig.assert_nothing_written_or_recorded();
        }
    }

    #[test]
    fn speech_model_failure_is_reported_and_nothing_written() {
        let rig = Rig {
            transcriber: FakeTranscriber::new(Err(TranscribeError::Failed("model missing".into()))),
            ..Rig::new("", Ok("x"))
        };

        let outcome = rig.run(&DICTATE, &cleanup_on());

        assert_eq!(
            outcome.status,
            Status::Problem(Problem::Other("model missing".into()))
        );
        assert_eq!(outcome.entry, None);
        rig.assert_nothing_written_or_recorded();
    }

    // ---- accessibility -----------------------------------------------------

    #[test]
    fn missing_accessibility_still_records_history() {
        let rig = Rig {
            inserter: FakeInserter::new(Err(Problem::AccessibilityPermission)),
            ..Rig::new("hello", Ok("Hello."))
        };

        let outcome = rig.run(&DICTATE, &cleanup_on());

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
        for action in [DICTATE, ENGLISH] {
            let rig = Rig {
                inserter: FakeInserter::new(Err(Problem::AccessibilityPermission)),
                ..Rig::new("hello", unreachable())
            };

            let outcome = rig.run(&action, &cleanup_on());

            assert_eq!(
                outcome.status,
                Status::Problem(Problem::AccessibilityPermission)
            );
        }
    }

    // ---- history -----------------------------------------------------------

    #[test]
    fn history_is_loaded_once_and_new_entries_go_first() {
        let rig = Rig {
            store: FakeStore::new(History::from(vec![old_entry(1)])),
            ..Rig::new("hello", Ok("Hello."))
        };
        let pipeline = rig.pipeline();

        pipeline.process(speech(), &DICTATE, &cleanup_on());
        pipeline.process(speech(), &ENGLISH, &cleanup_on());

        let saved = rig.last_saved();
        let shown: Vec<_> = saved
            .entries()
            .iter()
            .map(|e| (e.action, e.polished.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                (Action::DictateInEnglish, "Hello."),
                (Action::Dictate, "Hello."),
                (Action::Dictate, "Old 1."),
            ]
        );
        assert!(saved.entries()[0].at >= saved.entries()[1].at);
    }

    #[test]
    fn full_history_drops_the_oldest_when_an_action_finishes() {
        let full: Vec<_> = (1..=HISTORY_LIMIT).rev().map(old_entry).collect();
        let rig = Rig {
            store: FakeStore::new(History::from(full)),
            ..Rig::new("hello", Ok("Hello."))
        };

        rig.run(&DICTATE, &cleanup_on());

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

        let first = pipeline.process(speech(), &DICTATE, &cleanup_on());
        pipeline.process(speech(), &DICTATE, &cleanup_on());

        assert_eq!(first.status, Status::Done);
        assert!(first.entry.is_some());
        // the in-memory history keeps growing even though saving fails
        assert_eq!(rig.last_saved().entries().len(), 2);
    }

    impl Outcome {
        fn status_message(&self) -> String {
            match &self.status {
                Status::Problem(problem) => problem.to_string(),
                other => format!("{other:?}"),
            }
        }
    }
}
