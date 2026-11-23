//! `Transcriber` backed by a speech model served by the local AI app through the
//! OpenAI-compatible `/audio/transcriptions` endpoint.
use crate::multipart::Multipart;
use crate::text::{clean_transcript, initial_prompt};
use crate::wav::wav;
use std::io::Read;
use std::time::Duration;
use voice_core::{PersonalDictionary, Recording, TranscribeError, Transcriber};

/// How long one transcription may take before the server counts as unreachable.
pub const SERVER_TIMEOUT: Duration = Duration::from_secs(60);

/// Speech recognition by a model the local AI app serves.
#[derive(Debug, Clone)]
pub struct ServerTranscriber {
    base_url: String,
    model: String,
    timeout: Duration,
}

impl ServerTranscriber {
    /// `base_url` like `http://localhost:1234/v1`; `model` is the speech model's identifier.
    pub fn new(base_url: String, model: String) -> Self {
        Self {
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            model,
            timeout: SERVER_TIMEOUT,
        }
    }

    /// Overrides the overall request timeout (default [`SERVER_TIMEOUT`]).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn form(&self, recording: &Recording, dictionary: &PersonalDictionary) -> Multipart {
        let mut form = Multipart::new();
        form.file("file", "audio.wav", "audio/wav", &wav(&recording.samples));
        form.text("model", &self.model);
        form.text("response_format", "json");
        if let Some(prompt) = initial_prompt(dictionary) {
            form.text("prompt", &prompt);
        }
        form
    }
}

impl Transcriber for ServerTranscriber {
    fn transcribe(
        &self,
        recording: &Recording,
        dictionary: &PersonalDictionary,
    ) -> Result<String, TranscribeError> {
        if recording.samples.is_empty() {
            return Ok(String::new());
        }
        let form = self.form(recording, dictionary);
        let url = format!("{}/audio/transcriptions", self.base_url);
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .build()
            .into();
        let mut response = agent
            .post(&url)
            .header("Content-Type", form.content_type())
            .send(&form.finish()[..])
            .map_err(request_error)?;
        let mut body = String::new();
        response
            .body_mut()
            .as_reader()
            .read_to_string(&mut body)
            .map_err(|e| TranscribeError::Unreachable(format!("answer cut short: {e}")))?;
        let reply: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| TranscribeError::Failed(format!("unreadable speech model answer: {e}")))?;
        let text = reply["text"].as_str().ok_or_else(|| {
            TranscribeError::Failed("speech model answer has no text".to_string())
        })?;
        Ok(clean_transcript(&[text]))
    }
}

/// A server error (5xx) or no answer at all means the server is not usable right now;
/// any other refusal (4xx, e.g. unknown model) is a failure of this request.
fn request_error(e: ureq::Error) -> TranscribeError {
    match e {
        ureq::Error::StatusCode(code) if code < 500 => {
            TranscribeError::Failed(format!("speech model refused the recording (HTTP {code})"))
        }
        other => TranscribeError::Unreachable(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_server::*;
    use std::io::Cursor;

    fn transcriber(fake: &Fake) -> ServerTranscriber {
        ServerTranscriber::new(fake.base.clone(), "breeze-asr-25".into())
    }

    #[test]
    fn heard_words_come_from_the_chosen_speech_model() {
        let fake = serve(200, r#"{"text":" 我們開會 "}"#, Duration::ZERO);
        let out = transcriber(&fake).transcribe(&half_second(), &no_dict());
        assert_eq!(out, Ok("我們開會".to_string()));
        let seen = first(&fake);
        assert_eq!(seen.path, "/v1/audio/transcriptions");
        assert!(seen
            .content_type
            .starts_with("multipart/form-data; boundary="));
        assert_eq!(text_field(&seen, "model").as_deref(), Some("breeze-asr-25"));
        assert_eq!(
            text_field(&seen, "response_format").as_deref(),
            Some("json")
        );
    }

    #[test]
    fn audio_is_sent_as_16_bit_mono_wav_at_16_khz() {
        let fake = serve(200, r#"{"text":"hi"}"#, Duration::ZERO);
        let samples = vec![0.0, 0.5, -0.5, 1.0, -1.0, 2.0];
        transcriber(&fake)
            .transcribe(&Recording::new(samples), &no_dict())
            .expect("ok");
        let seen = first(&fake);
        let (headers, audio) = parts(&seen)
            .into_iter()
            .find(|(h, _)| h.contains("name=\"file\""))
            .expect("file part");
        assert!(headers.contains("filename=\"audio.wav\""), "{headers}");
        assert!(headers.contains("Content-Type: audio/wav"), "{headers}");
        assert_eq!(&audio[..4], b"RIFF");
        assert_eq!(&audio[8..12], b"WAVE");
        let reader = hound::WavReader::new(Cursor::new(audio)).expect("valid wav");
        let spec = reader.spec();
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(spec.sample_format, hound::SampleFormat::Int);
        let decoded: Vec<i16> = reader
            .into_samples::<i16>()
            .map(|s| s.expect("sample"))
            .collect();
        assert_eq!(decoded, vec![0, 16384, -16384, 32767, -32767, 32767]);
    }

    #[test]
    fn dictionary_words_are_sent_as_the_prompt() {
        let fake = serve(200, r#"{"text":"I work at Cafler"}"#, Duration::ZERO);
        let dictionary = PersonalDictionary::from_words(["Cafler", "Kubernetes"]);
        transcriber(&fake)
            .transcribe(&half_second(), &dictionary)
            .expect("ok");
        assert_eq!(
            text_field(&first(&fake), "prompt").as_deref(),
            Some("Cafler, Kubernetes")
        );
    }

    #[test]
    fn empty_dictionary_sends_no_prompt() {
        let fake = serve(200, r#"{"text":"hi"}"#, Duration::ZERO);
        transcriber(&fake)
            .transcribe(&half_second(), &no_dict())
            .expect("ok");
        assert_eq!(field(&first(&fake), "prompt"), None);
    }

    #[test]
    fn trailing_slash_in_base_url_is_tolerated() {
        let fake = serve(200, r#"{"text":"hi"}"#, Duration::ZERO);
        ServerTranscriber::new(format!("{}/ ", fake.base), "m".into())
            .transcribe(&half_second(), &no_dict())
            .expect("ok");
        assert_eq!(first(&fake).path, "/v1/audio/transcriptions");
    }

    #[test]
    fn silence_markers_give_an_empty_transcript() {
        let fake = serve(200, r#"{"text":" [BLANK_AUDIO] "}"#, Duration::ZERO);
        assert_eq!(
            transcriber(&fake).transcribe(&half_second(), &no_dict()),
            Ok(String::new())
        );
    }

    #[test]
    fn an_empty_recording_never_reaches_the_server() {
        let fake = serve(200, r#"{"text":"should not be used"}"#, Duration::ZERO);
        assert_eq!(
            transcriber(&fake).transcribe(&Recording::default(), &no_dict()),
            Ok(String::new())
        );
        assert!(fake.seen.lock().expect("lock").is_empty());
    }

    #[test]
    fn server_not_running_is_unreachable() {
        let t =
            ServerTranscriber::new(format!("http://127.0.0.1:{}/v1", unused_port()), "m".into());
        assert!(matches!(
            t.transcribe(&half_second(), &no_dict()),
            Err(TranscribeError::Unreachable(_))
        ));
    }

    #[test]
    fn server_errors_are_unreachable() {
        for status in [500, 503] {
            let fake = serve(status, "boom", Duration::ZERO);
            let out = transcriber(&fake).transcribe(&half_second(), &no_dict());
            assert!(
                matches!(out, Err(TranscribeError::Unreachable(_))),
                "{status}: {out:?}"
            );
        }
    }

    #[test]
    fn slow_answer_times_out_as_unreachable() {
        let fake = serve(200, r#"{"text":"late"}"#, Duration::from_millis(1500));
        let started = std::time::Instant::now();
        let out = transcriber(&fake)
            .with_timeout(Duration::from_millis(200))
            .transcribe(&half_second(), &no_dict());
        assert!(
            matches!(out, Err(TranscribeError::Unreachable(_))),
            "{out:?}"
        );
        assert!(started.elapsed() < Duration::from_millis(1400));
    }

    #[test]
    fn refused_request_is_a_failure() {
        for status in [400, 404, 422] {
            let fake = serve(status, r#"{"error":"unknown model"}"#, Duration::ZERO);
            let out = transcriber(&fake).transcribe(&half_second(), &no_dict());
            assert!(
                matches!(out, Err(TranscribeError::Failed(_))),
                "{status}: {out:?}"
            );
        }
    }

    #[test]
    fn unreadable_answer_is_a_failure() {
        for body in ["not json", r#"{"segments":[]}"#, r#"{"text":42}"#] {
            let fake = serve(200, body, Duration::ZERO);
            let out = transcriber(&fake).transcribe(&half_second(), &no_dict());
            assert!(
                matches!(out, Err(TranscribeError::Failed(_))),
                "{body}: {out:?}"
            );
        }
    }

    #[test]
    fn default_timeout_is_one_minute() {
        let t = ServerTranscriber::new("http://x/v1".into(), "m".into());
        assert_eq!(t.timeout, Duration::from_secs(60));
    }

    #[test]
    fn transcriber_is_shareable_between_threads() {
        fn assert_port<T: Transcriber>() {}
        assert_port::<ServerTranscriber>();
    }
}
