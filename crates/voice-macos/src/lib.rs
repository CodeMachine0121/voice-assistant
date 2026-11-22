//! macOS adapters: global shortcuts, text insertion at the cursor, and privacy permissions.

pub mod keys;

pub use keys::{KeyInput, KeyInterpreter, KeyKind, ShortcutConfig};
