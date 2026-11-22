//! macOS adapters: global shortcuts, text insertion at the cursor, and privacy permissions.

pub mod keys;
mod shortcuts;

pub use keys::{KeyInput, KeyInterpreter, KeyKind, ShortcutConfig};
pub use shortcuts::{EventTapShortcuts, ShortcutListener};
