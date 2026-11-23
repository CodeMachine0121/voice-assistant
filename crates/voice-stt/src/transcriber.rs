//! `Transcriber` implementation: turns a recording into the cleaned raw transcript.
use crate::text::{clean_transcript, initial_prompt};
use crate::whisper::WhisperEngine;
use std::path::PathBuf;
use voice_core::{PersonalDictionary, Recording, TranscribeError, Transcriber};

/// What the speech model offers: raw text segments for 16 kHz mono audio, optionally primed.
pub(crate) trait SpeechEngine: Send + Sync {
    fn segments(&self, samples: &[f32], prompt: Option<&str>) -> Result<Vec<String>, String>;
}

pub(crate) struct DictationTranscriber<E> {
    engine: E,
}

impl<E: SpeechEngine> DictationTranscriber<E> {
    pub(crate) fn new(engine: E) -> Self {
        Self { engine }
    }
}

impl<E: SpeechEngine> Transcriber for DictationTranscriber<E> {
    fn transcribe(
        &self,
        recording: &Recording,
        dictionary: &PersonalDictionary,
    ) -> Result<String, TranscribeError> {
        if recording.samples.is_empty() {
            return Ok(String::new());
        }
        let prompt = initial_prompt(dictionary);
        let segments = self
            .engine
            .segments(&recording.samples, prompt.as_deref())
            .map_err(TranscribeError::Failed)?;
        Ok(clean_transcript(&segments))
    }
}

/// Speech recognition with whisper.cpp (Metal) running on this Mac.
pub struct WhisperTranscriber(DictationTranscriber<WhisperEngine>);

impl WhisperTranscriber {
    /// Loads the model file once; every `transcribe` call reuses it.
    pub fn new(model_path: PathBuf) -> Result<Self, String> {
        Ok(Self(DictationTranscriber::new(WhisperEngine::load(
            &model_path,
        )?)))
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(
        &self,
        recording: &Recording,
        dictionary: &PersonalDictionary,
    ) -> Result<String, TranscribeError> {
        self.0.transcribe(recording, dictionary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeEngine {
        reply: Result<Vec<String>, String>,
        calls: Mutex<Vec<(usize, Option<String>)>>,
    }

    impl FakeEngine {
        fn replying(segments: &[&str]) -> Self {
            Self::with(Ok(segments.iter().map(|s| s.to_string()).collect()))
        }
        fn with(reply: Result<Vec<String>, String>) -> Self {
            Self {
                reply,
                calls: Mutex::default(),
            }
        }
    }

    impl SpeechEngine for FakeEngine {
        fn segments(&self, samples: &[f32], prompt: Option<&str>) -> Result<Vec<String>, String> {
            self.calls
                .lock()
                .unwrap()
                .push((samples.len(), prompt.map(String::from)));
            self.reply.clone()
        }
    }

    fn one_second() -> Recording {
        Recording::new(vec![0.1; 16_000])
    }

    #[test]
    fn dictionary_words_prime_the_model() {
        let t = DictationTranscriber::new(FakeEngine::replying(&[" I work at Cafler"]));
        let dictionary = PersonalDictionary::from_words(["Cafler", "Kubernetes"]);
        let text = t.transcribe(&one_second(), &dictionary).unwrap();
        assert_eq!(text, "I work at Cafler");
        assert_eq!(
            t.engine.calls.lock().unwrap()[0],
            (16_000, Some("Cafler, Kubernetes".to_string()))
        );
    }

    #[test]
    fn empty_dictionary_sends_no_prompt() {
        let t = DictationTranscriber::new(FakeEngine::replying(&[" hello"]));
        t.transcribe(&one_second(), &PersonalDictionary::default())
            .unwrap();
        assert_eq!(t.engine.calls.lock().unwrap()[0].1, None);
    }

    #[test]
    fn silence_markers_give_an_empty_transcript() {
        let t = DictationTranscriber::new(FakeEngine::replying(&[" [BLANK_AUDIO]"]));
        assert_eq!(
            t.transcribe(&one_second(), &PersonalDictionary::default()),
            Ok(String::new())
        );
    }

    #[test]
    fn an_empty_recording_never_reaches_the_model() {
        let t = DictationTranscriber::new(FakeEngine::replying(&["should not be used"]));
        assert_eq!(
            t.transcribe(&Recording::default(), &PersonalDictionary::default()),
            Ok(String::new())
        );
        assert!(t.engine.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn model_failure_is_a_failure_not_unreachable() {
        let engine = FakeEngine::with(Err("model exploded".into()));
        let t = DictationTranscriber::new(engine);
        assert_eq!(
            t.transcribe(&one_second(), &PersonalDictionary::default()),
            Err(TranscribeError::Failed("model exploded".to_string()))
        );
    }

    #[test]
    fn missing_model_file_is_a_load_error() {
        let err = WhisperTranscriber::new(PathBuf::from("/definitely/not/here.bin"))
            .err()
            .unwrap();
        assert!(err.contains("/definitely/not/here.bin"), "{err}");
    }

    #[test]
    fn file_that_is_not_a_model_is_a_load_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ggml-small.bin");
        std::fs::write(&path, b"not a model").unwrap();
        let err = WhisperTranscriber::new(path).err().unwrap();
        assert!(err.contains("cannot load speech model"), "{err}");
    }

    #[test]
    fn transcriber_is_shareable_between_threads() {
        fn assert_port<T: Transcriber>() {}
        assert_port::<WhisperTranscriber>();
    }
}
