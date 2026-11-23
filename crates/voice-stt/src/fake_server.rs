//! A local stand-in for a speech service, recording every request it receives.

use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tiny_http::{Response, Server};
use voice_core::{PersonalDictionary, Recording};

/// One request as the fake server saw it.
#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub(crate) path: String,
    pub(crate) content_type: String,
    pub(crate) authorization: Option<String>,
    pub(crate) body: Vec<u8>,
}

pub(crate) struct Fake {
    pub(crate) base: String,
    pub(crate) seen: Arc<Mutex<Vec<Seen>>>,
}

pub(crate) fn serve(status: u16, body: &'static str, delay: Duration) -> Fake {
    let server = Server::http("127.0.0.1:0").expect("bind test server");
    let base = format!(
        "http://127.0.0.1:{}/v1",
        server.server_addr().to_ip().expect("ip").port()
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    thread::spawn(move || {
        for mut req in server.incoming_requests() {
            let mut received = Vec::new();
            let _ = req.as_reader().read_to_end(&mut received);
            let content_type = req
                .headers()
                .iter()
                .find(|h| h.field.equiv("Content-Type"))
                .map(|h| h.value.to_string())
                .unwrap_or_default();
            let authorization = req
                .headers()
                .iter()
                .find(|h| h.field.equiv("Authorization"))
                .map(|h| h.value.to_string());
            log.lock().expect("lock").push(Seen {
                path: req.url().to_string(),
                content_type,
                authorization,
                body: received,
            });
            thread::sleep(delay);
            let _ = req.respond(Response::from_string(body).with_status_code(status));
        }
    });
    Fake { base, seen }
}

pub(crate) fn first(fake: &Fake) -> Seen {
    fake.seen.lock().expect("lock")[0].clone()
}

/// Splits a multipart body into (headers, content) per part.
pub(crate) fn parts(seen: &Seen) -> Vec<(String, Vec<u8>)> {
    let boundary = seen
        .content_type
        .split("boundary=")
        .nth(1)
        .expect("boundary in content type");
    let delimiter = format!("--{boundary}");
    let mut out = Vec::new();
    let body = &seen.body;
    let starts: Vec<usize> = body
        .windows(delimiter.len())
        .enumerate()
        .filter(|(_, w)| *w == delimiter.as_bytes())
        .map(|(i, _)| i)
        .collect();
    for pair in starts.windows(2) {
        let chunk = &body[pair[0] + delimiter.len() + 2..pair[1] - 2];
        let split = chunk
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("part headers end");
        out.push((
            String::from_utf8_lossy(&chunk[..split]).to_string(),
            chunk[split + 4..].to_vec(),
        ));
    }
    let last = *starts.last().expect("closing boundary");
    assert_eq!(&body[last + delimiter.len()..], b"--\r\n");
    out
}

pub(crate) fn field(seen: &Seen, name: &str) -> Option<Vec<u8>> {
    parts(seen)
        .into_iter()
        .find(|(headers, _)| headers.contains(&format!("name=\"{name}\"")))
        .map(|(_, content)| content)
}

pub(crate) fn text_field(seen: &Seen, name: &str) -> Option<String> {
    field(seen, name).map(|c| String::from_utf8(c).expect("utf8 field"))
}

pub(crate) fn half_second() -> Recording {
    Recording::new(vec![0.25; 8_000])
}

pub(crate) fn no_dict() -> PersonalDictionary {
    PersonalDictionary::default()
}

pub(crate) fn unused_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}
