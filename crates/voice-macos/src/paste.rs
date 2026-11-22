//! Text insertion by clipboard + ⌘V, putting the earlier clipboard text back.
//!
//! The order of operations lives in [`ClipboardPaste`], which talks to the
//! clipboard, the keyboard and the permission check through small traits so it
//! can be tested with fakes. The system implementations are thin wrappers.
use crate::permissions::MacPermissions;
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use std::time::Duration;
use voice_core::{Problem, TextInserter};

/// How long the front app gets to read the clipboard before it is restored.
const PASTE_SETTLE: Duration = Duration::from_millis(150);
/// Virtual key code of "V".
const KEY_V: u16 = 9;

trait Clipboard {
    /// Current text; `None` when the clipboard is empty or holds no text.
    fn text(&self) -> Option<String>;
    fn set_text(&self, text: &str) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
}

trait Keyboard {
    /// Sends ⌘V to the front app.
    fn paste(&self) -> Result<(), String>;
}

trait AccessCheck {
    fn accessibility_granted(&self) -> bool;
}

struct ClipboardPaste<C, K, A> {
    clipboard: C,
    keyboard: K,
    access: A,
    settle: Duration,
}

impl<C: Clipboard, K: Keyboard, A: AccessCheck> ClipboardPaste<C, K, A> {
    fn insert(&self, text: &str) -> Result<(), Problem> {
        if !self.access.accessibility_granted() {
            self.clipboard.set_text(text).map_err(clipboard_problem)?;
            return Err(Problem::AccessibilityPermission);
        }
        let previous = self.clipboard.text();
        self.clipboard.set_text(text).map_err(clipboard_problem)?;
        // On failure the text stays on the clipboard so the user can still paste it.
        self.keyboard.paste().map_err(|e| {
            Problem::Other(format!("Could not paste – text left on clipboard ({e})"))
        })?;
        std::thread::sleep(self.settle);
        let restored = match previous {
            Some(previous) => self.clipboard.set_text(&previous),
            None => self.clipboard.clear(),
        };
        if let Err(e) = restored {
            // The text was inserted; only the earlier clipboard content is lost.
            log::warn!("could not restore the clipboard: {e}");
        }
        Ok(())
    }
}

fn clipboard_problem(e: String) -> Problem {
    Problem::Other(format!("Clipboard unavailable ({e})"))
}

struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn text(&self) -> Option<String> {
        arboard::Clipboard::new()
            .and_then(|mut c| c.get_text())
            .ok()
            .filter(|t| !t.is_empty())
    }

    fn set_text(&self, text: &str) -> Result<(), String> {
        arboard::Clipboard::new()
            .and_then(|mut c| c.set_text(text))
            .map_err(|e| e.to_string())
    }

    fn clear(&self) -> Result<(), String> {
        arboard::Clipboard::new()
            .and_then(|mut c| c.clear())
            .map_err(|e| e.to_string())
    }
}

struct SystemKeyboard;

impl Keyboard for SystemKeyboard {
    fn paste(&self) -> Result<(), String> {
        let event = |down| {
            let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .map_err(|()| "no event source".to_string())?;
            let event = CGEvent::new_keyboard_event(source, KEY_V, down)
                .map_err(|()| "no keyboard event".to_string())?;
            event.set_flags(CGEventFlags::CGEventFlagCommand);
            Ok::<_, String>(event)
        };
        let (down, up) = (event(true)?, event(false)?);
        down.post(CGEventTapLocation::HID);
        up.post(CGEventTapLocation::HID);
        Ok(())
    }
}

impl AccessCheck for MacPermissions {
    fn accessibility_granted(&self) -> bool {
        MacPermissions::accessibility_granted()
    }
}

/// Inserts text at the cursor of the front app by pasting it, then puts the
/// earlier clipboard text back (or empties the clipboard if it held no text).
pub struct PasteInserter {
    inner: ClipboardPaste<SystemClipboard, SystemKeyboard, MacPermissions>,
}

impl PasteInserter {
    pub fn new() -> Self {
        Self {
            inner: ClipboardPaste {
                clipboard: SystemClipboard,
                keyboard: SystemKeyboard,
                access: MacPermissions,
                settle: PASTE_SETTLE,
            },
        }
    }
}

impl Default for PasteInserter {
    fn default() -> Self {
        Self::new()
    }
}

impl TextInserter for PasteInserter {
    fn insert(&self, text: &str) -> Result<(), Problem> {
        self.inner.insert(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeClipboard {
        content: RefCell<Option<String>>,
        broken: bool,
        clear_broken: bool,
    }

    impl FakeClipboard {
        fn current(&self) -> Option<String> {
            self.content.borrow().clone()
        }

        fn holding(text: &str) -> Self {
            Self {
                content: RefCell::new(Some(text.to_string())),
                broken: false,
                clear_broken: false,
            }
        }
    }

    impl Clipboard for &FakeClipboard {
        fn text(&self) -> Option<String> {
            self.content.borrow().clone()
        }

        fn set_text(&self, text: &str) -> Result<(), String> {
            if self.broken {
                return Err("pasteboard refused".into());
            }
            *self.content.borrow_mut() = Some(text.to_string());
            Ok(())
        }

        fn clear(&self) -> Result<(), String> {
            if self.clear_broken {
                return Err("pasteboard refused".into());
            }
            *self.content.borrow_mut() = None;
            Ok(())
        }
    }

    /// Records what was on the clipboard at each ⌘V.
    struct FakeKeyboard<'a> {
        clipboard: &'a FakeClipboard,
        pasted: RefCell<Vec<Option<String>>>,
        broken: bool,
    }

    impl<'a> FakeKeyboard<'a> {
        fn on(clipboard: &'a FakeClipboard) -> Self {
            Self {
                clipboard,
                pasted: RefCell::new(Vec::new()),
                broken: false,
            }
        }
    }

    impl Keyboard for &FakeKeyboard<'_> {
        fn paste(&self) -> Result<(), String> {
            if self.broken {
                return Err("no event source".into());
            }
            self.pasted.borrow_mut().push(self.clipboard.current());
            Ok(())
        }
    }

    struct Access(bool);

    impl AccessCheck for Access {
        fn accessibility_granted(&self) -> bool {
            self.0
        }
    }

    fn inserter<'a>(
        clipboard: &'a FakeClipboard,
        keyboard: &'a FakeKeyboard<'a>,
        granted: bool,
    ) -> ClipboardPaste<&'a FakeClipboard, &'a FakeKeyboard<'a>, Access> {
        ClipboardPaste {
            clipboard,
            keyboard,
            access: Access(granted),
            settle: Duration::ZERO,
        }
    }

    fn hello() -> Option<String> {
        Some("Hello.".to_string())
    }

    #[test]
    fn text_is_pasted_from_the_clipboard() {
        let clipboard = FakeClipboard::holding("invoice-42");
        let keyboard = FakeKeyboard::on(&clipboard);

        let result = inserter(&clipboard, &keyboard, true).insert("Hello.");

        assert_eq!((result, keyboard.pasted.take()), (Ok(()), vec![hello()]));
    }

    #[test]
    fn earlier_clipboard_text_is_restored() {
        let clipboard = FakeClipboard::holding("invoice-42");
        let keyboard = FakeKeyboard::on(&clipboard);

        inserter(&clipboard, &keyboard, true)
            .insert("Hello.")
            .unwrap();

        assert_eq!(clipboard.current(), Some("invoice-42".to_string()));
    }

    #[test]
    fn empty_clipboard_stays_empty() {
        let clipboard = FakeClipboard::default();
        let keyboard = FakeKeyboard::on(&clipboard);

        let result = inserter(&clipboard, &keyboard, true).insert("Hello.");

        assert_eq!(
            (result, keyboard.pasted.take(), clipboard.current()),
            (Ok(()), vec![hello()], None)
        );
    }

    #[test]
    fn missing_accessibility_leaves_text_on_clipboard() {
        let clipboard = FakeClipboard::holding("invoice-42");
        let keyboard = FakeKeyboard::on(&clipboard);

        let result = inserter(&clipboard, &keyboard, false).insert("Hello.");

        assert_eq!(
            (result, keyboard.pasted.take(), clipboard.current()),
            (Err(Problem::AccessibilityPermission), vec![], hello())
        );
    }

    #[test]
    fn clipboard_failure_is_reported_without_pasting() {
        for granted in [true, false] {
            let clipboard = FakeClipboard {
                broken: true,
                ..FakeClipboard::default()
            };
            let keyboard = FakeKeyboard::on(&clipboard);

            let result = inserter(&clipboard, &keyboard, granted).insert("Hello.");

            assert!(
                matches!(result, Err(Problem::Other(_))) && keyboard.pasted.take().is_empty(),
                "granted={granted}: {result:?}"
            );
        }
    }

    #[test]
    fn paste_failure_leaves_text_on_clipboard() {
        let clipboard = FakeClipboard::holding("invoice-42");
        let keyboard = FakeKeyboard {
            broken: true,
            ..FakeKeyboard::on(&clipboard)
        };

        let result = inserter(&clipboard, &keyboard, true).insert("Hello.");

        assert!(
            matches!(result, Err(Problem::Other(_))) && clipboard.current() == hello(),
            "{result:?}"
        );
    }

    #[test]
    fn failing_to_restore_the_clipboard_still_counts_as_inserted() {
        let clipboard = FakeClipboard {
            clear_broken: true,
            ..FakeClipboard::default()
        };
        let keyboard = FakeKeyboard::on(&clipboard);

        let result = inserter(&clipboard, &keyboard, true).insert("Hello.");

        assert_eq!(result, Ok(()));
    }
}
