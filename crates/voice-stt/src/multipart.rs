//! A `multipart/form-data` request body.
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

/// A `multipart/form-data` body built by hand (the HTTP client has no helper for it).
pub(crate) struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    /// The boundary carries 128 random bits, so it practically never occurs in the audio.
    pub(crate) fn new() -> Self {
        let random = || RandomState::new().build_hasher().finish();
        Self {
            boundary: format!("----voice-assistant-{:016x}{:016x}", random(), random()),
            body: Vec::new(),
        }
    }

    pub(crate) fn content_type(&self) -> String {
        format!("multipart/form-data; boundary={}", self.boundary)
    }

    pub(crate) fn text(&mut self, name: &str, value: &str) {
        self.part(
            &format!("Content-Disposition: form-data; name=\"{name}\""),
            value.as_bytes(),
        );
    }

    pub(crate) fn file(&mut self, name: &str, filename: &str, content_type: &str, data: &[u8]) {
        self.part(
            &format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n\
                 Content-Type: {content_type}"
            ),
            data,
        );
    }

    fn part(&mut self, headers: &str, data: &[u8]) {
        self.body
            .extend_from_slice(format!("--{}\r\n{headers}\r\n\r\n", self.boundary).as_bytes());
        self.body.extend_from_slice(data);
        self.body.extend_from_slice(b"\r\n");
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.body
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        self.body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_differ_between_requests() {
        assert_ne!(Multipart::new().boundary, Multipart::new().boundary);
    }
}
