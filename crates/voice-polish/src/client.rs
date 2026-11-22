//! HTTP client for LM Studio (OpenAI-compatible API).
use crate::clean::clean_answer;
use crate::prompt::{build_messages, ChatMessage};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use voice_core::{rules, PersonalDictionary, PolishError, Polisher};

const TEMPERATURE: f64 = 0.2;

/// `Polisher` backed by LM Studio's `/chat/completions` endpoint.
#[derive(Debug, Clone)]
pub struct LmStudioPolisher {
    base_url: String,
    model: String,
    timeout: Duration,
}

impl LmStudioPolisher {
    /// `base_url` like `http://localhost:1234/v1`; an empty `model` uses whatever is loaded.
    pub fn new(base_url: String, model: String) -> Self {
        Self {
            base_url: normalize(&base_url),
            model,
            timeout: rules::ASSISTANT_TIMEOUT,
        }
    }

    /// Overrides the overall request timeout (default `rules::ASSISTANT_TIMEOUT`).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn request_body(&self, messages: &[ChatMessage]) -> Value {
        let mut body = json!({
            "messages": messages,
            "temperature": TEMPERATURE,
            "stream": false,
        });
        if !self.model.trim().is_empty() {
            body["model"] = Value::String(self.model.clone());
        }
        body
    }
}

impl Polisher for LmStudioPolisher {
    fn polish(&self, raw: &str, dictionary: &PersonalDictionary) -> Result<String, PolishError> {
        let body = self.request_body(&build_messages(raw, dictionary));
        let url = format!("{}/chat/completions", self.base_url);
        let reply: Reply = agent(self.timeout)
            .post(&url)
            .send_json(&body)
            .map_err(unreachable)?
            .body_mut()
            .read_json()
            .map_err(|e| PolishError::InvalidResponse(format!("unreadable answer: {e}")))?;
        let content = reply
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .map(|c| clean_answer(&c))
            .unwrap_or_default();
        if content.is_empty() {
            return Err(PolishError::InvalidResponse("empty answer".into()));
        }
        Ok(content)
    }
}

/// Lists the model identifiers the assistant offers, for the settings screen.
pub fn list_models(base_url: &str) -> Result<Vec<String>, String> {
    let url = format!("{}/models", normalize(base_url));
    let list: ModelList = agent(rules::ASSISTANT_TIMEOUT)
        .get(&url)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_json()
        .map_err(|e| format!("unreadable model list: {e}"))?;
    Ok(list.data.into_iter().map(|m| m.id).collect())
}

fn normalize(base_url: &str) -> String {
    base_url.trim().trim_end_matches('/').to_string()
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into()
}

fn unreachable(e: ureq::Error) -> PolishError {
    PolishError::Unreachable(e.to_string())
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Deserialize)]
struct Message {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ModelList {
    #[serde(default)]
    data: Vec<ModelInfo>,
}

#[derive(Deserialize)]
struct ModelInfo {
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use tiny_http::{Response, Server};

    /// Serves one canned response per request and records request bodies.
    struct Fake {
        base: String,
        bodies: Arc<Mutex<Vec<String>>>,
        paths: Arc<Mutex<Vec<String>>>,
    }

    fn serve(status: u16, body: &'static str, delay: Duration) -> Fake {
        let server = Server::http("127.0.0.1:0").expect("bind test server");
        let base = format!(
            "http://127.0.0.1:{}/v1",
            server.server_addr().to_ip().expect("ip").port()
        );
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let (b, p) = (bodies.clone(), paths.clone());
        thread::spawn(move || {
            for mut req in server.incoming_requests() {
                let mut s = String::new();
                let _ = req.as_reader().read_to_string(&mut s);
                b.lock().expect("lock").push(s);
                p.lock().expect("lock").push(req.url().to_string());
                thread::sleep(delay);
                let _ = req.respond(Response::from_string(body).with_status_code(status));
            }
        });
        Fake {
            base,
            bodies,
            paths,
        }
    }

    fn chat(content: &str) -> &'static str {
        let s = json!({"choices":[{"message":{"role":"assistant","content":content}}]}).to_string();
        Box::leak(s.into_boxed_str())
    }

    fn polisher(fake: &Fake) -> LmStudioPolisher {
        LmStudioPolisher::new(fake.base.clone(), "qwen".into())
    }

    fn no_dict() -> PersonalDictionary {
        PersonalDictionary::default()
    }

    #[test]
    fn returns_cleaned_text_on_success() {
        let fake = serve(200, chat("The meeting is at 4 tomorrow."), Duration::ZERO);
        let out = polisher(&fake).polish("um meeting at 4", &no_dict());
        assert_eq!(out, Ok("The meeting is at 4 tomorrow.".to_string()));
        assert_eq!(fake.paths.lock().expect("lock")[0], "/v1/chat/completions");
    }

    #[test]
    fn request_carries_model_temperature_no_stream_and_transcript() {
        let fake = serve(200, chat("ok"), Duration::ZERO);
        polisher(&fake)
            .polish("hello there", &no_dict())
            .expect("ok");
        let sent: Value =
            serde_json::from_str(&fake.bodies.lock().expect("lock")[0]).expect("json");
        assert_eq!(sent["model"], "qwen");
        assert_eq!(sent["temperature"], 0.2);
        assert_eq!(sent["stream"], false);
        let msgs = sent["messages"].as_array().expect("messages");
        assert_eq!(
            msgs.last().expect("last")["content"],
            "<transcript>hello there</transcript>"
        );
    }

    #[test]
    fn dictionary_words_reach_the_request() {
        let fake = serve(200, chat("ok"), Duration::ZERO);
        let mut d = PersonalDictionary::default();
        d.add("Cafler");
        polisher(&fake).polish("caffler", &d).expect("ok");
        assert!(fake.bodies.lock().expect("lock")[0].contains("Always spell these exactly: Cafler"));
    }

    #[test]
    fn empty_model_is_omitted() {
        let fake = serve(200, chat("ok"), Duration::ZERO);
        LmStudioPolisher::new(fake.base.clone(), String::new())
            .polish("x", &no_dict())
            .expect("ok");
        let sent: Value =
            serde_json::from_str(&fake.bodies.lock().expect("lock")[0]).expect("json");
        assert!(sent.get("model").is_none());
    }

    #[test]
    fn trailing_slash_in_base_url_is_tolerated() {
        let fake = serve(200, chat("ok"), Duration::ZERO);
        let p = LmStudioPolisher::new(format!("{}/", fake.base), "m".into());
        assert!(p.polish("x", &no_dict()).is_ok());
        assert_eq!(fake.paths.lock().expect("lock")[0], "/v1/chat/completions");
    }

    #[test]
    fn strips_reasoning_quotes_and_fences() {
        let fake = serve(200, chat("<think>plan</think>\n\"Hello.\""), Duration::ZERO);
        assert_eq!(
            polisher(&fake).polish("hello", &no_dict()),
            Ok("Hello.".into())
        );
    }

    #[test]
    fn empty_content_is_invalid_response() {
        for body in [
            chat(""),
            chat("   "),
            chat("<think>only thoughts</think>"),
            r#"{"choices":[{"message":{"content":null}}]}"#,
            r#"{"choices":[]}"#,
            r#"{}"#,
        ] {
            let fake = serve(200, body, Duration::ZERO);
            let out = polisher(&fake).polish("x", &no_dict());
            assert!(
                matches!(out, Err(PolishError::InvalidResponse(_))),
                "{body}: {out:?}"
            );
        }
    }

    #[test]
    fn unparsable_body_is_invalid_response() {
        let fake = serve(200, "not json", Duration::ZERO);
        let out = polisher(&fake).polish("x", &no_dict());
        assert!(matches!(out, Err(PolishError::InvalidResponse(_))));
    }

    #[test]
    fn non_success_status_is_unreachable() {
        for status in [404, 500, 503] {
            let fake = serve(status, "boom", Duration::ZERO);
            let out = polisher(&fake).polish("x", &no_dict());
            assert!(
                matches!(out, Err(PolishError::Unreachable(_))),
                "{status}: {out:?}"
            );
        }
    }

    #[test]
    fn connection_refused_is_unreachable() {
        let port = TcpListener::bind("127.0.0.1:0")
            .expect("bind")
            .local_addr()
            .expect("addr")
            .port();
        let p = LmStudioPolisher::new(format!("http://127.0.0.1:{port}/v1"), "m".into());
        assert!(matches!(
            p.polish("x", &no_dict()),
            Err(PolishError::Unreachable(_))
        ));
    }

    #[test]
    fn slow_answer_times_out_as_unreachable() {
        let fake = serve(200, chat("late"), Duration::from_millis(1500));
        let p = polisher(&fake).with_timeout(Duration::from_millis(200));
        let started = std::time::Instant::now();
        let out = p.polish("x", &no_dict());
        assert!(matches!(out, Err(PolishError::Unreachable(_))), "{out:?}");
        assert!(started.elapsed() < Duration::from_millis(1400));
    }

    #[test]
    fn default_timeout_is_the_business_rule() {
        let p = LmStudioPolisher::new("http://x/v1".into(), String::new());
        assert_eq!(p.timeout, rules::ASSISTANT_TIMEOUT);
    }

    #[test]
    fn lists_model_ids() {
        let fake = serve(
            200,
            r#"{"data":[{"id":"qwen2.5-7b-instruct"},{"id":"llama-3"}]}"#,
            Duration::ZERO,
        );
        assert_eq!(
            list_models(&fake.base),
            Ok(vec![
                "qwen2.5-7b-instruct".to_string(),
                "llama-3".to_string()
            ])
        );
        assert_eq!(fake.paths.lock().expect("lock")[0], "/v1/models");
    }

    #[test]
    fn list_models_without_data_is_empty() {
        let fake = serve(200, "{}", Duration::ZERO);
        assert_eq!(list_models(&format!("{}/", fake.base)), Ok(vec![]));
    }

    #[test]
    fn list_models_reports_failures() {
        let fake = serve(500, "x", Duration::ZERO);
        assert!(list_models(&fake.base).is_err());
        let fake = serve(200, "garbage", Duration::ZERO);
        assert!(list_models(&fake.base).is_err());
    }
}
