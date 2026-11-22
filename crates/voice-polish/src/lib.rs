//! Writing-assistant adapter: polishes transcripts through LM Studio's
//! OpenAI-compatible chat API.
mod clean;
mod client;
mod prompt;

pub use client::{list_models, LmStudioPolisher};
pub use prompt::{build_messages, ChatMessage};
