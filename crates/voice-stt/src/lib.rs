//! Speech recognition: the `Transcriber` port on top of whisper.cpp running on this Mac,
//! or of a speech model served by the local AI app.
//!
//! Prompt building, transcript cleaning and model download bookkeeping are pure/fake-tested;
//! only the thin [`whisper`] module talks to the real model.

pub mod model;
mod server;
pub mod text;
mod transcriber;
mod whisper;

pub use model::{default_model_path, download_default_model};
pub use server::{ServerTranscriber, SERVER_TIMEOUT};
pub use transcriber::WhisperTranscriber;
