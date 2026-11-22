//! Drawing. Thin by design: everything that can be decided without a window lives
//! in the view-model modules; this layer only turns it into egui calls.
pub mod history;
pub mod indicator;
pub mod settings;
