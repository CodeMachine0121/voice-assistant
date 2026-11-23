//! `Transcriber` backed by OpenAI's speech-to-text service
//! (`POST {base}/audio/transcriptions`).
use crate::key::OpenAiKey;
use crate::multipart::Multipart;
use crate::text::{clean_transcript, initial_prompt};
use crate::wav::wav;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;
use voice_core::{PersonalDictionary, Recording, TranscribeError, Transcriber};

/// Where OpenAI's API lives.
pub const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
/// How long one transcription may take before OpenAI counts as unreachable.
pub const OPENAI_TIMEOUT: Duration = Duration::from_secs(60);

/// Speech recognition by an OpenAI speech model.
#[derive(Debug, Clone)]
pub struct OpenAiTranscriber {
    base_url: String,
    model: String,
    key: Arc<OpenAiKey>,
    timeout: Duration,
}

impl OpenAiTranscriber {
    /// `model` is OpenAI's identifier of the speech model, e.g. `gpt-4o-transcribe`.
    /// The key is looked up again for every recording.
    pub fn new(model: String, key: Arc<OpenAiKey>) -> Self {
        Self {
            base_url: OPENAI_BASE_URL.to_string(),
            model,
            key,
            timeout: OPENAI_TIMEOUT,
        }
    }

    /// Talks to another OpenAI-compatible address (default [`OPENAI_BASE_URL`]).
    pub fn with_base_url(mut self, base_url: &str) -> Self {
        self.base_url = base_url.trim().trim_end_matches('/').to_string();
        self
    }

    /// Overrides the overall request timeout (default [`OPENAI_TIMEOUT`]).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The spoken language is left for OpenAI to detect, so mixed-language speech works.
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

impl Transcriber for OpenAiTranscriber {
    fn transcribe(
        &self,
        recording: &Recording,
        dictionary: &PersonalDictionary,
    ) -> Result<String, TranscribeError> {
        if recording.samples.is_empty() {
            return Ok(String::new());
        }
        let key = self.key.current().ok_or(TranscribeError::KeyMissing)?;
        let form = self.form(recording, dictionary);
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .build()
            .into();
        let mut response = agent
            .post(&format!("{}/audio/transcriptions", self.base_url))
            .header("Authorization", &format!("Bearer {key}"))
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
            .map_err(|e| TranscribeError::Failed(format!("unreadable OpenAI answer: {e}")))?;
        let text = reply["text"]
            .as_str()
            .ok_or_else(|| TranscribeError::Failed("OpenAI answer has no text".to_string()))?;
        Ok(clean_transcript(&[text]))
    }
}

/// 401 = key not accepted, 429 = rate limit or quota, other 4xx = this recording was
/// refused; 5xx, timeouts and connection failures mean OpenAI is not usable right now.
fn request_error(e: ureq::Error) -> TranscribeError {
    match e {
        ureq::Error::StatusCode(401) => TranscribeError::KeyRejected,
        ureq::Error::StatusCode(429) => TranscribeError::LimitReached,
        ureq::Error::StatusCode(code) if code < 500 => {
            TranscribeError::Failed(format!("OpenAI refused the recording (HTTP {code})"))
        }
        other => TranscribeError::Unreachable(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_server::*;
    use crate::key::MemorySecretStore;
    use std::io::Cursor;

    fn key(saved: &str) -> Arc<OpenAiKey> {
        Arc::new(OpenAiKey::with_environment(
            Arc::new(MemorySecretStore::holding(saved)),
            || None,
        ))
    }

    fn transcriber(fake: &Fake) -> OpenAiTranscriber {
        OpenAiTranscriber::new("gpt-4o-transcribe".into(), key("sk-test")).with_base_url(&fake.base)
    }

    #[test]
    fn heard_words_come_from_the_chosen_openai_speech_model() {
        let fake = serve(200, r#"{"text":"我們明天下午三點開會"}"#, Duration::ZERO);

        let out = transcriber(&fake).transcribe(&half_second(), &no_dict());

        assert_eq!(out, Ok("我們明天下午三點開會".to_string()));
        let seen = fake.seen.lock().expect("lock").clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].path, "/v1/audio/transcriptions");
        assert_eq!(seen[0].authorization.as_deref(), Some("Bearer sk-test"));
        assert_eq!(
            text_field(&seen[0], "model").as_deref(),
            Some("gpt-4o-transcribe")
        );
        assert_eq!(
            text_field(&seen[0], "response_format").as_deref(),
            Some("json")
        );
    }

    #[test]
    fn the_configured_model_is_the_one_asked_for() {
        let fake = serve(200, r#"{"text":"hi"}"#, Duration::ZERO);

        OpenAiTranscriber::new("gpt-4o-mini-transcribe".into(), key("sk-test"))
            .with_base_url(&fake.base)
            .transcribe(&half_second(), &no_dict())
            .expect("ok");

        assert_eq!(
            text_field(&first(&fake), "model").as_deref(),
            Some("gpt-4o-mini-transcribe")
        );
    }

    #[test]
    fn dictionary_words_are_sent_as_a_spelling_hint_and_no_language_is_named() {
        let fake = serve(200, r#"{"text":"I work at Cafler"}"#, Duration::ZERO);

        transcriber(&fake)
            .transcribe(&half_second(), &PersonalDictionary::from_words(["Cafler"]))
            .expect("ok");

        let seen = first(&fake);
        assert_eq!(text_field(&seen, "prompt").as_deref(), Some("Cafler"));
        assert_eq!(field(&seen, "language"), None);
    }

    #[test]
    fn an_empty_dictionary_sends_no_prompt() {
        let fake = serve(200, r#"{"text":"hi"}"#, Duration::ZERO);

        transcriber(&fake)
            .transcribe(&half_second(), &no_dict())
            .expect("ok");

        assert_eq!(field(&first(&fake), "prompt"), None);
    }

    #[test]
    fn an_empty_recording_is_never_sent() {
        let fake = serve(200, r#"{"text":"should not be used"}"#, Duration::ZERO);

        let out = transcriber(&fake).transcribe(&Recording::default(), &no_dict());

        assert_eq!(out, Ok(String::new()));
        assert!(fake.seen.lock().expect("lock").is_empty());
    }

    #[test]
    fn a_five_minute_recording_is_sent_whole() {
        let fake = serve(200, r#"{"text":"long talk"}"#, Duration::ZERO);
        let five_minutes = Recording::new(vec![0.1; 4_800_000]);

        let out = transcriber(&fake).transcribe(&five_minutes, &no_dict());

        assert_eq!(out, Ok("long talk".to_string()));
        let audio = field(&first(&fake), "file").expect("file part");
        assert_eq!(audio.len(), 44 + 9_600_000);
        let reader = hound::WavReader::new(Cursor::new(audio)).expect("valid wav");
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.len(), 4_800_000);
    }

    #[test]
    fn without_any_key_nothing_is_sent() {
        let fake = serve(200, r#"{"text":"hi"}"#, Duration::ZERO);
        let no_key = Arc::new(OpenAiKey::with_environment(
            Arc::new(MemorySecretStore::default()),
            || None,
        ));

        let out = OpenAiTranscriber::new("gpt-4o-transcribe".into(), no_key)
            .with_base_url(&fake.base)
            .transcribe(&half_second(), &no_dict());

        assert_eq!(out, Err(TranscribeError::KeyMissing));
        assert!(fake.seen.lock().expect("lock").is_empty());
    }

    #[test]
    fn a_rejected_key_is_reported_as_such() {
        let fake = serve(
            401,
            r#"{"error":{"code":"invalid_api_key"}}"#,
            Duration::ZERO,
        );

        let out = transcriber(&fake).transcribe(&half_second(), &no_dict());

        assert_eq!(out, Err(TranscribeError::KeyRejected));
    }

    #[test]
    fn too_many_requests_or_no_credit_is_a_reached_limit() {
        let fake = serve(
            429,
            r#"{"error":{"code":"insufficient_quota"}}"#,
            Duration::ZERO,
        );

        let out = transcriber(&fake).transcribe(&half_second(), &no_dict());

        assert_eq!(out, Err(TranscribeError::LimitReached));
    }

    #[test]
    fn no_connection_is_unreachable() {
        let t = OpenAiTranscriber::new("gpt-4o-transcribe".into(), key("sk-test"))
            .with_base_url(&format!("http://127.0.0.1:{}/v1", unused_port()));

        let out = t.transcribe(&half_second(), &no_dict());

        assert!(
            matches!(out, Err(TranscribeError::Unreachable(_))),
            "{out:?}"
        );
    }

    #[test]
    fn trouble_on_openai_side_is_unreachable() {
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
    fn an_answer_within_the_time_limit_is_accepted() {
        let fake = serve(200, r#"{"text":"我們開會"}"#, Duration::from_millis(100));

        let out = transcriber(&fake)
            .with_timeout(Duration::from_millis(1_000))
            .transcribe(&half_second(), &no_dict());

        assert_eq!(out, Ok("我們開會".to_string()));
    }

    #[test]
    fn no_answer_within_the_time_limit_is_unreachable() {
        let fake = serve(200, r#"{"text":"late"}"#, Duration::from_millis(1_500));
        let started = std::time::Instant::now();

        let out = transcriber(&fake)
            .with_timeout(Duration::from_millis(200))
            .transcribe(&half_second(), &no_dict());

        assert!(
            matches!(out, Err(TranscribeError::Unreachable(_))),
            "{out:?}"
        );
        assert!(started.elapsed() < Duration::from_millis(1_400));
    }

    #[test]
    fn the_time_limit_is_one_minute_and_the_address_is_openai() {
        let t = OpenAiTranscriber::new("m".into(), key("sk-test"));

        assert_eq!(t.timeout, Duration::from_secs(60));
        assert_eq!(t.base_url, "https://api.openai.com/v1");
    }

    #[test]
    fn other_refusals_mean_the_recording_was_not_turned_into_words() {
        for status in [400, 403, 404] {
            let fake = serve(
                status,
                r#"{"error":{"code":"model_not_found"}}"#,
                Duration::ZERO,
            );

            let out = transcriber(&fake).transcribe(&half_second(), &no_dict());

            assert!(
                matches!(out, Err(TranscribeError::Failed(_))),
                "{status}: {out:?}"
            );
        }
    }

    #[test]
    fn unusable_answers_mean_the_recording_was_not_turned_into_words() {
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
    fn an_answer_cut_short_is_unreachable() {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{\"te");
            }
        });

        let out = OpenAiTranscriber::new("m".into(), key("sk-test"))
            .with_base_url(&format!("http://127.0.0.1:{port}/v1"))
            .transcribe(&half_second(), &no_dict());

        assert!(
            matches!(out, Err(TranscribeError::Unreachable(_))),
            "{out:?}"
        );
    }

    #[test]
    fn silence_markers_give_an_empty_transcript() {
        let fake = serve(200, r#"{"text":" [BLANK_AUDIO] "}"#, Duration::ZERO);

        let out = transcriber(&fake).transcribe(&half_second(), &no_dict());

        assert_eq!(out, Ok(String::new()));
    }

    #[test]
    fn errors_never_contain_the_key() {
        for status in [400, 401, 429, 500] {
            let fake = serve(status, "sk-test", Duration::ZERO);

            let err = transcriber(&fake)
                .transcribe(&half_second(), &no_dict())
                .expect_err("refused");

            assert!(!format!("{err} {err:?}").contains("sk-test"), "{err:?}");
        }
    }

    #[test]
    fn trailing_slash_in_the_address_is_tolerated() {
        let fake = serve(200, r#"{"text":"hi"}"#, Duration::ZERO);

        OpenAiTranscriber::new("m".into(), key("sk-test"))
            .with_base_url(&format!("{}/ ", fake.base))
            .transcribe(&half_second(), &no_dict())
            .expect("ok");

        assert_eq!(first(&fake).path, "/v1/audio/transcriptions");
    }

    #[test]
    fn transcriber_is_shareable_between_threads() {
        fn assert_port<T: Transcriber>() {}
        assert_port::<OpenAiTranscriber>();
    }
}
