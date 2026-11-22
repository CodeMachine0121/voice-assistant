//! Text insertion by clipboard + ⌘V, putting the earlier clipboard content back.
//!
//! The order of operations lives in [`ClipboardPaste`], which talks to the
//! clipboard, the keyboard and the permission check through small traits so it
//! can be tested with fakes. The system implementations are thin wrappers.
use crate::permissions::MacPermissions;
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeString, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSData, NSString};
use std::time::Duration;
use voice_core::{Problem, TextInserter};

/// How long the front app gets to read the clipboard before it is restored.
const PASTE_SETTLE: Duration = Duration::from_millis(150);
/// Virtual key code of "V".
const KEY_V: u16 = 9;

/// One clipboard item: every representation it offered, as (type, bytes).
type ClipboardItem = Vec<(String, Vec<u8>)>;

/// Everything the clipboard held at one moment. No items means an empty clipboard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ClipboardSnapshot {
    items: Vec<ClipboardItem>,
}

trait Clipboard {
    fn snapshot(&self) -> ClipboardSnapshot;
    /// Replaces the clipboard content with `text` and returns the change count
    /// the clipboard has right after that write.
    fn set_text(&self, text: &str) -> Result<isize, String>;
    /// A counter every write to the clipboard bumps, whichever app makes it.
    fn change_count(&self) -> isize;
    /// Replaces the clipboard content with `snapshot`.
    fn restore(&self, snapshot: ClipboardSnapshot) -> Result<(), String>;
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
        let previous = self.clipboard.snapshot();
        let ours = self.clipboard.set_text(text).map_err(clipboard_problem)?;
        // On failure the text stays on the clipboard so the user can still paste it.
        self.keyboard.paste().map_err(|e| {
            Problem::Other(format!("Could not paste – text left on clipboard ({e})"))
        })?;
        std::thread::sleep(self.settle);
        if self.clipboard.change_count() != ours {
            // Someone copied something meanwhile; putting the old content back would erase it.
            log::info!("clipboard changed during the paste; leaving it as it is");
            return Ok(());
        }
        if let Err(e) = self.clipboard.restore(previous) {
            // The text was inserted; only the earlier clipboard content is lost.
            log::warn!("could not restore the clipboard: {e}");
        }
        Ok(())
    }
}

fn clipboard_problem(e: String) -> Problem {
    Problem::Other(format!("Clipboard unavailable ({e})"))
}

/// The general pasteboard, through AppKit.
struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn snapshot(&self) -> ClipboardSnapshot {
        autoreleasepool(|_| {
            let items = NSPasteboard::generalPasteboard()
                .pasteboardItems()
                .map(|items| items.iter().map(|item| item_data(&item)).collect())
                .unwrap_or_default();
            ClipboardSnapshot { items }
        })
        .without_empty_items()
    }

    fn set_text(&self, text: &str) -> Result<isize, String> {
        autoreleasepool(|_| {
            let pasteboard = NSPasteboard::generalPasteboard();
            pasteboard.clearContents();
            // SAFETY: an immutable string constant exported by AppKit.
            let string_type = unsafe { NSPasteboardTypeString };
            if !pasteboard.setString_forType(&NSString::from_str(text), string_type) {
                return Err("pasteboard refused the text".to_string());
            }
            Ok(pasteboard.changeCount())
        })
    }

    fn change_count(&self) -> isize {
        autoreleasepool(|_| NSPasteboard::generalPasteboard().changeCount())
    }

    fn restore(&self, snapshot: ClipboardSnapshot) -> Result<(), String> {
        autoreleasepool(|_| {
            let items = snapshot
                .items
                .into_iter()
                .map(pasteboard_item)
                .collect::<Result<Vec<_>, _>>()?;
            let pasteboard = NSPasteboard::generalPasteboard();
            pasteboard.clearContents();
            if items.is_empty() || pasteboard.writeObjects(&NSArray::from_retained_slice(&items)) {
                Ok(())
            } else {
                Err("pasteboard refused the earlier content".to_string())
            }
        })
    }
}

impl ClipboardSnapshot {
    /// Drops items that offered no readable data, so they do not restore as blanks.
    fn without_empty_items(mut self) -> Self {
        self.items.retain(|item| !item.is_empty());
        self
    }
}

/// Every type the item offers whose data can be read right now.
fn item_data(item: &NSPasteboardItem) -> ClipboardItem {
    item.types()
        .iter()
        .filter_map(|kind| {
            let data = item.dataForType(&kind)?;
            Some((kind.to_string(), data.to_vec()))
        })
        .collect()
}

fn pasteboard_item(
    representations: ClipboardItem,
) -> Result<Retained<ProtocolObject<dyn NSPasteboardWriting>>, String> {
    let item = NSPasteboardItem::new();
    for (kind, bytes) in representations {
        if !item.setData_forType(&NSData::with_bytes(&bytes), &NSString::from_str(&kind)) {
            return Err(format!("pasteboard item refused data of type {kind}"));
        }
    }
    Ok(ProtocolObject::from_retained(item))
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

/// Inserts text at the cursor of the front app by pasting it, then puts back
/// whatever the clipboard held before (text, images, files, rich text – or nothing).
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
    use std::cell::{Cell, RefCell};

    const TEXT: &str = "public.utf8-plain-text";
    const PNG: &str = "public.png";
    const RTF: &str = "public.rtf";

    fn item(representations: &[(&str, &[u8])]) -> ClipboardItem {
        representations
            .iter()
            .map(|(kind, bytes)| (kind.to_string(), bytes.to_vec()))
            .collect()
    }

    fn snapshot(items: Vec<ClipboardItem>) -> ClipboardSnapshot {
        ClipboardSnapshot { items }
    }

    fn text(text: &str) -> ClipboardSnapshot {
        snapshot(vec![item(&[(TEXT, text.as_bytes())])])
    }

    #[derive(Default)]
    struct FakeClipboard {
        content: RefCell<ClipboardSnapshot>,
        changes: Cell<isize>,
        broken: bool,
        restore_broken: bool,
    }

    impl FakeClipboard {
        fn holding(content: ClipboardSnapshot) -> Self {
            Self {
                content: RefCell::new(content),
                ..Self::default()
            }
        }

        fn current(&self) -> ClipboardSnapshot {
            self.content.borrow().clone()
        }

        /// Another app copying something.
        fn write(&self, content: ClipboardSnapshot) {
            *self.content.borrow_mut() = content;
            self.changes.set(self.changes.get() + 1);
        }
    }

    impl Clipboard for &FakeClipboard {
        fn snapshot(&self) -> ClipboardSnapshot {
            self.current()
        }

        fn set_text(&self, value: &str) -> Result<isize, String> {
            if self.broken {
                return Err("pasteboard refused".into());
            }
            self.write(text(value));
            Ok(self.changes.get())
        }

        fn change_count(&self) -> isize {
            self.changes.get()
        }

        fn restore(&self, content: ClipboardSnapshot) -> Result<(), String> {
            if self.restore_broken {
                return Err("pasteboard refused".into());
            }
            self.write(content);
            Ok(())
        }
    }

    /// Records what was on the clipboard at each ⌘V.
    struct FakeKeyboard<'a> {
        clipboard: &'a FakeClipboard,
        pasted: RefCell<Vec<ClipboardSnapshot>>,
        broken: bool,
        /// Copied by another app right after the paste.
        copied_meanwhile: Option<ClipboardSnapshot>,
    }

    impl<'a> FakeKeyboard<'a> {
        fn on(clipboard: &'a FakeClipboard) -> Self {
            Self {
                clipboard,
                pasted: RefCell::new(Vec::new()),
                broken: false,
                copied_meanwhile: None,
            }
        }
    }

    impl Keyboard for &FakeKeyboard<'_> {
        fn paste(&self) -> Result<(), String> {
            if self.broken {
                return Err("no event source".into());
            }
            self.pasted.borrow_mut().push(self.clipboard.current());
            if let Some(other) = &self.copied_meanwhile {
                self.clipboard.write(other.clone());
            }
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

    fn hello() -> ClipboardSnapshot {
        text("Hello.")
    }

    #[test]
    fn text_is_pasted_from_the_clipboard() {
        let clipboard = FakeClipboard::holding(text("invoice-42"));
        let keyboard = FakeKeyboard::on(&clipboard);

        let result = inserter(&clipboard, &keyboard, true).insert("Hello.");

        assert_eq!((result, keyboard.pasted.take()), (Ok(()), vec![hello()]));
    }

    #[test]
    fn earlier_clipboard_text_is_restored() {
        let clipboard = FakeClipboard::holding(text("invoice-42"));
        let keyboard = FakeKeyboard::on(&clipboard);

        inserter(&clipboard, &keyboard, true)
            .insert("Hello.")
            .unwrap();

        assert_eq!(clipboard.current(), text("invoice-42"));
    }

    #[test]
    fn earlier_image_is_restored_byte_for_byte() {
        let image: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff];
        let before = snapshot(vec![item(&[(PNG, image)])]);
        let clipboard = FakeClipboard::holding(before.clone());
        let keyboard = FakeKeyboard::on(&clipboard);

        let result = inserter(&clipboard, &keyboard, true).insert("Hello.");

        assert_eq!(
            (result, keyboard.pasted.take(), clipboard.current()),
            (Ok(()), vec![hello()], before)
        );
    }

    #[test]
    fn every_item_and_representation_is_restored() {
        let before = snapshot(vec![
            item(&[(RTF, b"{\\rtf1 \\b bold}"), (TEXT, b"bold")]),
            item(&[("public.file-url", b"file:///Users/me/a.pdf")]),
            item(&[("public.file-url", b"file:///Users/me/b.pdf")]),
        ]);
        let clipboard = FakeClipboard::holding(before.clone());
        let keyboard = FakeKeyboard::on(&clipboard);

        inserter(&clipboard, &keyboard, true)
            .insert("Hello.")
            .unwrap();

        assert_eq!(clipboard.current(), before);
    }

    #[test]
    fn empty_clipboard_stays_empty() {
        let clipboard = FakeClipboard::default();
        let keyboard = FakeKeyboard::on(&clipboard);

        let result = inserter(&clipboard, &keyboard, true).insert("Hello.");

        assert_eq!(
            (result, keyboard.pasted.take(), clipboard.current()),
            (Ok(()), vec![hello()], ClipboardSnapshot::default())
        );
    }

    #[test]
    fn content_copied_during_the_paste_is_kept() {
        let clipboard = FakeClipboard::holding(text("invoice-42"));
        let keyboard = FakeKeyboard {
            copied_meanwhile: Some(text("copied meanwhile")),
            ..FakeKeyboard::on(&clipboard)
        };

        let result = inserter(&clipboard, &keyboard, true).insert("Hello.");

        assert_eq!(
            (result, keyboard.pasted.take(), clipboard.current()),
            (Ok(()), vec![hello()], text("copied meanwhile"))
        );
    }

    #[test]
    fn missing_accessibility_leaves_text_on_clipboard() {
        let clipboard = FakeClipboard::holding(snapshot(vec![item(&[(PNG, &[1, 2, 3])])]));
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
        let clipboard = FakeClipboard::holding(text("invoice-42"));
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
            restore_broken: true,
            ..FakeClipboard::holding(text("invoice-42"))
        };
        let keyboard = FakeKeyboard::on(&clipboard);

        let result = inserter(&clipboard, &keyboard, true).insert("Hello.");

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn items_without_readable_data_are_left_out() {
        let read = snapshot(vec![Vec::new(), item(&[(PNG, &[7])]), Vec::new()]);

        assert_eq!(
            read.without_empty_items(),
            snapshot(vec![item(&[(PNG, &[7])])])
        );
    }

    /// Touches the real clipboard; run by hand with `--ignored`. The user's
    /// clipboard is put back afterwards.
    #[test]
    #[ignore = "uses the real system clipboard"]
    fn system_clipboard_round_trips_custom_types() {
        let system = SystemClipboard;
        let users = system.snapshot();
        let custom = snapshot(vec![
            item(&[
                ("com.voice-assistant.test", &[0, 1, 2, 254, 255]),
                (TEXT, b"marker"),
            ]),
            item(&[("com.voice-assistant.test", b"second item")]),
        ]);

        system.restore(custom.clone()).unwrap();
        let taken = system.snapshot();
        let ours = system.set_text("overwritten").unwrap();
        let count_matches = system.change_count() == ours;
        let overwritten = system.snapshot();
        system.restore(taken.clone()).unwrap();
        let restored = system.snapshot();
        system.restore(ClipboardSnapshot::default()).unwrap();
        let emptied = system.snapshot();
        system.restore(users).unwrap();

        assert_eq!(taken, custom);
        assert!(count_matches);
        assert_eq!(overwritten, text("overwritten"));
        assert_eq!(restored, custom);
        assert_eq!(emptied, ClipboardSnapshot::default());
    }
}
