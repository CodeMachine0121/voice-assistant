//! macOS adapters: global shortcuts, reading the selected text, text insertion at
//! the cursor, and privacy permissions.

pub mod keys;
mod paste;
mod permissions;
mod selection;
mod service;
mod shortcuts;

pub use keys::{KeyDecision, KeyInput, KeyInterpreter, KeyKind, ShortcutConfig};
pub use paste::PasteInserter;
pub use permissions::{MacPermissions, PrivacyPane};
pub use selection::ClipboardSelectionReader;
pub use service::{ShortcutHandle, ShortcutService, RETRY_INTERVAL};
pub use shortcuts::{EventTapShortcuts, ShortcutListener};
