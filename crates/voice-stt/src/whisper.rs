//! The only code that touches whisper.cpp: model loading and one inference call.
use crate::transcriber::SpeechEngine;
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// The model needs at least one second of sound; shorter takes are padded with silence.
const MIN_SAMPLES: usize = 16_000;

/// Segments the model itself rates as more likely noise than speech are discarded.
const NO_SPEECH_LIMIT: f32 = 0.6;

pub(crate) struct WhisperEngine {
    context: WhisperContext,
    threads: i32,
}

impl WhisperEngine {
    pub(crate) fn load(model_path: &Path) -> Result<Self, String> {
        let path = model_path
            .to_str()
            .ok_or_else(|| format!("model path is not valid text: {}", model_path.display()))?;
        if !model_path.is_file() {
            return Err(format!("speech model not found at {path}"));
        }
        let context = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map_err(|e| format!("cannot load speech model {path}: {e}"))?;
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()) as i32;
        Ok(Self { context, threads })
    }
}

/// Pads `samples` with trailing silence up to the model's minimum length.
pub(crate) fn padded(samples: &[f32]) -> Vec<f32> {
    let mut audio = samples.to_vec();
    if audio.len() < MIN_SAMPLES {
        audio.resize(MIN_SAMPLES, 0.0);
    }
    audio
}

impl SpeechEngine for WhisperEngine {
    fn segments(&self, samples: &[f32], prompt: Option<&str>) -> Result<Vec<String>, String> {
        let audio = padded(samples);
        let mut state = self
            .context
            .create_state()
            .map_err(|e| format!("cannot start speech recognition: {e}"))?;

        // Greedy decoding keeps a 15 s dictation well under the latency budget.
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(self.threads);
        params.set_language(Some("auto"));
        params.set_translate(false);
        params.set_no_timestamps(true);
        params.set_no_context(true);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        if let Some(prompt) = prompt {
            params.set_initial_prompt(prompt);
        }

        state
            .full(params, &audio)
            .map_err(|e| format!("speech recognition failed: {e}"))?;
        Ok(state
            .as_iter()
            .filter(|s| s.no_speech_probability() <= NO_SPEECH_LIMIT)
            .map(|s| s.to_str_lossy().map(|t| t.into_owned()).unwrap_or_default())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_takes_are_padded_to_one_second_with_silence() {
        let audio = padded(&[0.5; 100]);
        assert_eq!(audio.len(), 16_000);
        assert!(audio[..100].iter().all(|&s| s == 0.5));
        assert!(audio[100..].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn long_takes_are_left_alone() {
        assert_eq!(padded(&[0.5; 20_000]).len(), 20_000);
    }

    /// Model for the ignored tests: `VOICE_STT_MODEL`, else the default location.
    fn model_for_tests() -> Option<std::path::PathBuf> {
        let path = std::env::var_os("VOICE_STT_MODEL")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(crate::default_model_path);
        path.exists().then_some(path)
    }

    /// Needs a downloaded model: `cargo test -p voice-stt -- --ignored`.
    #[test]
    #[ignore]
    fn real_model_hears_silence_as_nothing() {
        use voice_core::{PersonalDictionary, Recording, Transcriber};
        let Some(path) = model_for_tests() else {
            eprintln!("no speech model available, skipping");
            return;
        };
        let transcriber = crate::WhisperTranscriber::new(path).expect("model loads");
        let silence = Recording::new(vec![0.0; 32_000]);
        let text = transcriber
            .transcribe(&silence, &PersonalDictionary::default())
            .unwrap();
        assert_eq!(text, "");
    }

    /// Needs a model and a 16 kHz mono WAV in `VOICE_STT_SAMPLE` saying "ask not".
    #[test]
    #[ignore]
    fn real_model_transcribes_speech() {
        use voice_core::{PersonalDictionary, Recording, Transcriber};
        let (Some(path), Some(wav)) = (model_for_tests(), std::env::var_os("VOICE_STT_SAMPLE"))
        else {
            eprintln!("no model or sample available, skipping");
            return;
        };
        let samples: Vec<f32> = hound::WavReader::open(wav)
            .expect("wav opens")
            .into_samples::<i16>()
            .map(|s| s.expect("sample") as f32 / 32768.0)
            .collect();
        let transcriber = crate::WhisperTranscriber::new(path).expect("model loads");
        let text = transcriber
            .transcribe(&Recording::new(samples), &PersonalDictionary::default())
            .unwrap();
        assert!(text.to_lowercase().contains("ask not"), "{text}");
    }
}
