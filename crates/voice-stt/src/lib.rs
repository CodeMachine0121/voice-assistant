//! On-device speech recognition: the `Transcriber` port on top of whisper.cpp.
//!
//! Prompt building, transcript cleaning and model download bookkeeping are pure/fake-tested;
//! only the thin [`whisper`] module talks to the real model.

pub mod model;
pub mod text;
mod transcriber;
mod whisper;

pub use model::{default_model_path, download_default_model};
pub use transcriber::WhisperTranscriber;
