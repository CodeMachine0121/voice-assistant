//! macOS adapters: global shortcuts, reading the selected text, text insertion at
//! the cursor, privacy permissions, secrets in the Keychain and bringing the app to
//! the front.

mod activation;
mod keychain;
pub mod keys;
mod paste;
mod permissions;
mod selection;
mod service;
mod shortcuts;
mod surroundings;

pub use activation::{activate_app, return_focus};
pub use keychain::{KeychainSecret, KEYCHAIN_SERVICE, OPENAI_KEY_ACCOUNT};
pub use keys::{
    KeyDecision, KeyInput, KeyInterpreter, KeyKind, ShortcutConfig, Surroundings, MAX_TAP_HOLD,
};
pub use paste::PasteInserter;
pub use permissions::{MacPermissions, PrivacyPane};
pub use selection::ClipboardSelectionReader;
pub use service::{ShortcutHandle, ShortcutService, RETRY_INTERVAL};
pub use shortcuts::{EventTapShortcuts, ShortcutListener};
