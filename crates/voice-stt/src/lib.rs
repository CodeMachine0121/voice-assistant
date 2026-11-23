//! Speech recognition: the `Transcriber` port on top of OpenAI's speech-to-text service,
//! plus the OpenAI key it authenticates with.
//!
//! Prompt building and transcript cleaning are pure; the HTTP adapter is tested against
//! a local fake server.

#[cfg(test)]
mod fake_server;
mod key;
mod multipart;
mod openai;
pub mod text;
mod wav;

pub use key::{KeyStatus, MemorySecretStore, OpenAiKey, SecretStore, OPENAI_KEY_VARIABLE};
pub use openai::{OpenAiTimeouts, OpenAiTranscriber, OPENAI_BASE_URL, OPENAI_TIMEOUTS};
