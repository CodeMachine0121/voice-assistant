//! Writing-assistant adapter: carries out a `WritingTask` (clean up, translate to
//! English, edit a selection) through LM Studio's OpenAI-compatible chat API.
mod clean;
mod client;
mod prompt;

pub use client::{list_models, LmStudioPolisher};
pub use prompt::{messages, ChatMessage};
