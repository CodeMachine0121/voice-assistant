//! Fake adapters shared by the unit tests of this crate.
use crate::rules::SAMPLE_RATE;
use crate::{
    History, HistoryStore, PersonalDictionary, PolishError, Polisher, Problem, Recording,
    SelectionReader, TextInserter, TranscribeError, Transcriber, WritingTask,
};
use std::sync::{Arc, Mutex};

/// One second of a loud square wave: clearly speech.
pub fn speech() -> Recording {
    Recording::new(
        (0..SAMPLE_RATE)
            .map(|i| if i % 2 == 0 { 0.3 } else { -0.3 })
            .collect(),
    )
}

/// One second of digital silence.
pub fn silence() -> Recording {
    Recording::new(vec![0.0; SAMPLE_RATE as usize])
}

pub struct FakeTranscriber {
    answer: Result<String, TranscribeError>,
    calls: Mutex<Vec<PersonalDictionary>>,
}

impl FakeTranscriber {
    pub fn new(answer: Result<&str, TranscribeError>) -> Arc<Self> {
        Arc::new(Self {
            answer: answer.map(str::to_string),
            calls: Mutex::new(Vec::new()),
        })
    }
    pub fn calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
    pub fn dictionaries(&self) -> Vec<PersonalDictionary> {
        self.calls.lock().unwrap().clone()
    }
}

impl Transcriber for FakeTranscriber {
    fn transcribe(&self, _: &Recording, d: &PersonalDictionary) -> Result<String, TranscribeError> {
        self.calls.lock().unwrap().push(d.clone());
        self.answer.clone()
    }
}

/// An owned copy of a `WritingTask`, as the fake writing assistant received it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Task {
    CleanUp(String),
    TranslateToEnglish(String),
    EditSelection {
        instruction: String,
        selection: String,
    },
}

impl From<&WritingTask<'_>> for Task {
    fn from(task: &WritingTask<'_>) -> Self {
        match *task {
            WritingTask::CleanUp { heard } => Task::CleanUp(heard.into()),
            WritingTask::TranslateToEnglish { heard } => Task::TranslateToEnglish(heard.into()),
            WritingTask::EditSelection {
                instruction,
                selection,
            } => Task::EditSelection {
                instruction: instruction.into(),
                selection: selection.into(),
            },
        }
    }
}

pub struct FakePolisher {
    answer: Result<String, PolishError>,
    calls: Mutex<Vec<(Task, PersonalDictionary)>>,
}

impl FakePolisher {
    pub fn new(answer: Result<&str, PolishError>) -> Arc<Self> {
        Arc::new(Self {
            answer: answer.map(str::to_string),
            calls: Mutex::new(Vec::new()),
        })
    }
    pub fn calls(&self) -> Vec<(Task, PersonalDictionary)> {
        self.calls.lock().unwrap().clone()
    }
    pub fn tasks(&self) -> Vec<Task> {
        self.calls().into_iter().map(|(task, _)| task).collect()
    }
}

impl Polisher for FakePolisher {
    fn polish(
        &self,
        task: &WritingTask<'_>,
        d: &PersonalDictionary,
    ) -> Result<String, PolishError> {
        self.calls.lock().unwrap().push((task.into(), d.clone()));
        self.answer.clone()
    }
}

pub struct FakeSelectionReader {
    answer: Result<Option<String>, Problem>,
    calls: Mutex<usize>,
}

impl FakeSelectionReader {
    pub fn new(answer: Result<Option<&str>, Problem>) -> Arc<Self> {
        Arc::new(Self {
            answer: answer.map(|text| text.map(str::to_string)),
            calls: Mutex::new(0),
        })
    }
    pub fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl SelectionReader for FakeSelectionReader {
    fn selected_text(&self) -> Result<Option<String>, Problem> {
        *self.calls.lock().unwrap() += 1;
        self.answer.clone()
    }
}

pub struct FakeInserter {
    result: Result<(), Problem>,
    inserted: Mutex<Vec<String>>,
}

impl FakeInserter {
    pub fn new(result: Result<(), Problem>) -> Arc<Self> {
        Arc::new(Self {
            result,
            inserted: Mutex::new(Vec::new()),
        })
    }
    pub fn inserted(&self) -> Vec<String> {
        self.inserted.lock().unwrap().clone()
    }
}

impl TextInserter for FakeInserter {
    fn insert(&self, text: &str) -> Result<(), Problem> {
        self.inserted.lock().unwrap().push(text.to_string());
        self.result.clone()
    }
}

pub struct FakeStore {
    initial: History,
    fail_save: bool,
    saved: Mutex<Vec<History>>,
}

impl FakeStore {
    pub fn new(initial: History) -> Arc<Self> {
        Arc::new(Self {
            initial,
            fail_save: false,
            saved: Mutex::new(Vec::new()),
        })
    }
    pub fn failing() -> Arc<Self> {
        Arc::new(Self {
            initial: History::default(),
            fail_save: true,
            saved: Mutex::new(Vec::new()),
        })
    }
    pub fn saved(&self) -> Vec<History> {
        self.saved.lock().unwrap().clone()
    }
}

impl HistoryStore for FakeStore {
    fn load(&self) -> History {
        self.initial.clone()
    }
    fn save(&self, h: &History) -> Result<(), String> {
        self.saved.lock().unwrap().push(h.clone());
        if self.fail_save {
            Err("disk full".into())
        } else {
            Ok(())
        }
    }
}
