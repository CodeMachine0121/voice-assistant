//! Fake adapters shared by the unit tests of this crate.
use crate::rules::SAMPLE_RATE;
use crate::{
    History, HistoryStore, PersonalDictionary, PolishError, Polisher, Problem, Recording,
    TextInserter, Transcriber,
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
    answer: Result<String, String>,
    calls: Mutex<Vec<PersonalDictionary>>,
}

impl FakeTranscriber {
    pub fn new(answer: Result<&str, &str>) -> Arc<Self> {
        Arc::new(Self {
            answer: answer.map(str::to_string).map_err(str::to_string),
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
    fn transcribe(&self, _: &Recording, d: &PersonalDictionary) -> Result<String, String> {
        self.calls.lock().unwrap().push(d.clone());
        self.answer.clone()
    }
}

pub struct FakePolisher {
    answer: Result<String, PolishError>,
    calls: Mutex<Vec<(String, PersonalDictionary)>>,
}

impl FakePolisher {
    pub fn new(answer: Result<&str, PolishError>) -> Arc<Self> {
        Arc::new(Self {
            answer: answer.map(str::to_string),
            calls: Mutex::new(Vec::new()),
        })
    }
    pub fn calls(&self) -> Vec<(String, PersonalDictionary)> {
        self.calls.lock().unwrap().clone()
    }
}

impl Polisher for FakePolisher {
    fn polish(&self, raw: &str, d: &PersonalDictionary) -> Result<String, PolishError> {
        self.calls
            .lock()
            .unwrap()
            .push((raw.to_string(), d.clone()));
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
