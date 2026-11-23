//! Speech recognition: the `Transcriber` port on top of OpenAI's speech-to-text service,
//! plus the OpenAI key it authenticates with.
//!
//! Prompt building and transcript cleaning are pure; the HTTP adapter is tested against
//! a local fake server.

#[cfg(test)]
mod fake_server;
mod key;
pub mod model;
mod multipart;
mod openai;
mod server;
pub mod text;
mod transcriber;
mod wav;
mod whisper;

pub use key::{KeyStatus, MemorySecretStore, OpenAiKey, SecretStore, OPENAI_KEY_VARIABLE};
pub use model::{default_model_path, download_default_model};
pub use openai::{OpenAiTranscriber, OPENAI_BASE_URL, OPENAI_TIMEOUT};
pub use server::{ServerTranscriber, SERVER_TIMEOUT};
pub use transcriber::WhisperTranscriber;
