//! Reading the text selected in the front app.
//!
//! Accessibility is asked first: most native apps expose the selected text of the
//! focused element directly, and an empty selection reported there means nothing
//! is selected. Apps that do not expose it (many browsers and Electron apps) are
//! sent ⌘C; the copied text is read and the clipboard is put back as it was.
//! VS Code copies the whole line when nothing is selected; it marks such a copy,
//! and it then counts as nothing selected.
//! The order of operations lives in [`SelectionCopy`], tested with fakes; the
//! system implementations are thin wrappers.
use crate::paste::{
    AccessCheck, Clipboard, ClipboardSnapshot, Keyboard, SystemClipboard, SystemKeyboard,
};
use crate::permissions::MacPermissions;
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use std::time::{Duration, Instant};
use voice_core::{Problem, SelectionReader};

/// How long the front app gets to put the copied selection on the clipboard.
const COPY_WAIT: Duration = Duration::from_millis(200);
/// How often the clipboard is checked while waiting for the copy.
const COPY_POLL: Duration = Duration::from_millis(10);
/// Upper bound on how long a busy front app may keep an Accessibility query waiting.
/// Kept short so reading the selection stays within about 0.3 s even when the copy
/// fallback is needed; an app too slow for it just takes the fallback.
const AX_TIMEOUT_SECONDS: f32 = 0.1;

/// The pasteboard type VS Code adds to its copies, with JSON describing them.
const VSCODE_EDITOR_DATA: &str = "vscode-editor-data";

/// The selected text of the focused element, as Accessibility reports it.
pub(crate) trait FocusedSelection {
    /// `None` when there is no focused element or it does not expose its selection;
    /// `Some("")` when it exposes it and nothing is selected.
    fn selected_text(&self) -> Option<String>;
}

/// VS Code copied the line under the cursor because nothing was selected.
fn copied_from_empty_selection(copied: &ClipboardSnapshot) -> bool {
    copied
        .data(VSCODE_EDITOR_DATA)
        .and_then(|json| serde_json::from_slice::<serde_json::Value>(json).ok())
        .and_then(|data| data.get("isFromEmptySelection")?.as_bool())
        .unwrap_or(false)
}

struct SelectionCopy<F, C, K, A> {
    focused: F,
    clipboard: C,
    keyboard: K,
    access: A,
    copy_wait: Duration,
}

impl<F: FocusedSelection, C: Clipboard, K: Keyboard, A: AccessCheck> SelectionCopy<F, C, K, A> {
    fn selected_text(&self) -> Result<Option<String>, Problem> {
        if !self.access.accessibility_granted() {
            return Err(Problem::AccessibilityPermission);
        }
        if let Some(text) = self.focused.selected_text() {
            // The app exposes its selection: an empty one means nothing is selected.
            return Ok(Some(text).filter(|t| !t.is_empty()));
        }
        self.copy_selection()
    }

    /// Sends ⌘C and reads what lands on the clipboard, then puts the earlier content back.
    fn copy_selection(&self) -> Result<Option<String>, Problem> {
        let previous = self.clipboard.snapshot();
        let before = self.clipboard.change_count();
        self.keyboard
            .copy()
            .map_err(|e| Problem::Other(format!("Could not copy the selected text ({e})")))?;
        let Some(copied) = self.wait_for_copy(before) else {
            // Nothing was copied, so nothing selected; the clipboard was not touched.
            return Ok(None);
        };
        self.put_back(previous, copied.change_count);
        if copied_from_empty_selection(&copied.content) {
            return Ok(None);
        }
        Ok(copied.content.text().filter(|t| !t.is_empty()))
    }

    /// Waits for the copy to land on the clipboard. Apps clear the clipboard and
    /// write to it in separate steps, so after the first change this keeps looking
    /// until plain text is there or the copy wait is over. `None` when the
    /// clipboard never changed.
    fn wait_for_copy(&self, before: isize) -> Option<Copied> {
        let deadline = Instant::now() + self.copy_wait;
        let mut latest = None;
        loop {
            let change_count = self.clipboard.change_count();
            if change_count != before {
                let content = self.clipboard.snapshot();
                let has_text = content.text().is_some();
                latest = Some(Copied {
                    content,
                    change_count,
                });
                if has_text {
                    return latest;
                }
            }
            if Instant::now() >= deadline {
                return latest;
            }
            std::thread::sleep(COPY_POLL);
        }
    }

    /// Restores the earlier clipboard content, unless something was copied after our ⌘C.
    fn put_back(&self, previous: ClipboardSnapshot, copied_change_count: isize) {
        if self.clipboard.change_count() != copied_change_count {
            // Putting the old content back would erase what was copied meanwhile.
            log::info!("clipboard changed after copying the selection; leaving it as it is");
            return;
        }
        if let Err(e) = self.clipboard.restore(previous) {
            log::warn!("could not restore the clipboard after copying the selection: {e}");
        }
    }
}

/// What the clipboard held once the copy landed, and its change count at that moment.
struct Copied {
    content: ClipboardSnapshot,
    change_count: isize,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateSystemWide() -> CFTypeRef;
    fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetMessagingTimeout(element: CFTypeRef, seconds: f32) -> i32;
}

/// `kAXErrorSuccess`.
const AX_SUCCESS: i32 = 0;

/// Reads one attribute of an Accessibility element; `None` on any error.
fn ax_attribute(element: &CFType, attribute: &str) -> Option<CFType> {
    let name = CFString::new(attribute);
    let mut value: CFTypeRef = std::ptr::null();
    // SAFETY: `element` is a live AXUIElement, `name` a live CFString, and `value`
    // a valid out pointer; the call follows the Create rule for `value`.
    let error = unsafe {
        AXUIElementCopyAttributeValue(
            element.as_CFTypeRef(),
            name.as_concrete_TypeRef(),
            &mut value,
        )
    };
    if error != AX_SUCCESS || value.is_null() {
        return None;
    }
    // SAFETY: a non-null value returned under the Create rule; we own it now.
    Some(unsafe { CFType::wrap_under_create_rule(value) })
}

/// The focused element of the front app, through the Accessibility API.
pub(crate) struct SystemFocusedSelection;

impl FocusedSelection for SystemFocusedSelection {
    fn selected_text(&self) -> Option<String> {
        // SAFETY: plain constructor; the returned element follows the Create rule.
        let system = unsafe { AXUIElementCreateSystemWide() };
        if system.is_null() {
            return None;
        }
        // SAFETY: non-null and owned by us (Create rule).
        let system = unsafe { CFType::wrap_under_create_rule(system) };
        // SAFETY: `system` is a live AXUIElement. A failure only keeps the default timeout.
        unsafe { AXUIElementSetMessagingTimeout(system.as_CFTypeRef(), AX_TIMEOUT_SECONDS) };
        let focused = ax_attribute(&system, "AXFocusedUIElement")?;
        let selected = ax_attribute(&focused, "AXSelectedText")?;
        selected.downcast_into::<CFString>().map(|s| s.to_string())
    }
}

/// Reads the front app's selected text without disturbing the clipboard:
/// Accessibility first, ⌘C with the clipboard put back afterwards as a fallback.
pub struct ClipboardSelectionReader {
    inner: SelectionCopy<SystemFocusedSelection, SystemClipboard, SystemKeyboard, MacPermissions>,
}

impl ClipboardSelectionReader {
    pub fn new() -> Self {
        Self {
            inner: SelectionCopy {
                focused: SystemFocusedSelection,
                clipboard: SystemClipboard,
                keyboard: SystemKeyboard,
                access: MacPermissions,
                copy_wait: COPY_WAIT,
            },
        }
    }
}

impl Default for ClipboardSelectionReader {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectionReader for ClipboardSelectionReader {
    fn selected_text(&self) -> Result<Option<String>, Problem> {
        self.inner.selected_text()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paste::ClipboardItem;
    use std::cell::{Cell, RefCell};

    const TEXT: &str = "public.utf8-plain-text";
    const PNG: &str = "public.png";

    fn text(value: &str) -> ClipboardSnapshot {
        ClipboardSnapshot {
            items: vec![vec![(TEXT.to_string(), value.as_bytes().to_vec())]],
        }
    }

    fn image() -> ClipboardSnapshot {
        let item: ClipboardItem = vec![(PNG.to_string(), vec![0x89, b'P', b'N', b'G'])];
        ClipboardSnapshot { items: vec![item] }
    }

    struct Focused(Option<&'static str>);

    impl FocusedSelection for Focused {
        fn selected_text(&self) -> Option<String> {
            self.0.map(str::to_string)
        }
    }

    #[derive(Default)]
    struct FakeClipboard {
        content: RefCell<ClipboardSnapshot>,
        changes: Cell<isize>,
        restore_broken: bool,
        /// How often the change count was read; stands in for time passing.
        reads: Cell<usize>,
        /// Writes that land when the change count is read for the n-th time.
        scheduled: RefCell<Vec<(usize, ClipboardSnapshot)>>,
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

        fn write(&self, content: ClipboardSnapshot) {
            *self.content.borrow_mut() = content;
            self.changes.set(self.changes.get() + 1);
        }

        /// `content` is written just before the change count is read for the `read`-th time.
        fn schedule(&self, read: usize, content: ClipboardSnapshot) {
            self.scheduled.borrow_mut().push((read, content));
        }
    }

    impl Clipboard for &FakeClipboard {
        fn snapshot(&self) -> ClipboardSnapshot {
            self.current()
        }

        fn set_text(&self, value: &str) -> Result<isize, String> {
            self.write(text(value));
            Ok(self.changes.get())
        }

        fn change_count(&self) -> isize {
            self.reads.set(self.reads.get() + 1);
            let due: Vec<_> = {
                let mut scheduled = self.scheduled.borrow_mut();
                let (due, later) = scheduled
                    .drain(..)
                    .partition(|(read, _)| *read <= self.reads.get());
                *scheduled = later;
                due
            };
            for (_, content) in due {
                self.write(content);
            }
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

    /// The front app: on ⌘C it copies its selection, if it has one.
    struct FrontApp<'a> {
        clipboard: &'a FakeClipboard,
        selection: Option<ClipboardSnapshot>,
        copies: Cell<usize>,
        broken: bool,
    }

    impl<'a> FrontApp<'a> {
        fn selecting(clipboard: &'a FakeClipboard, selection: Option<ClipboardSnapshot>) -> Self {
            Self {
                clipboard,
                selection,
                copies: Cell::new(0),
                broken: false,
            }
        }
    }

    impl Keyboard for &FrontApp<'_> {
        fn paste(&self) -> Result<(), String> {
            Err("not used when reading the selection".into())
        }

        fn copy(&self) -> Result<(), String> {
            if self.broken {
                return Err("no event source".into());
            }
            self.copies.set(self.copies.get() + 1);
            if let Some(selection) = &self.selection {
                self.clipboard.write(selection.clone());
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

    fn reader<'a>(
        focused: Option<&'static str>,
        clipboard: &'a FakeClipboard,
        app: &'a FrontApp<'a>,
        granted: bool,
    ) -> SelectionCopy<Focused, &'a FakeClipboard, &'a FrontApp<'a>, Access> {
        SelectionCopy {
            focused: Focused(focused),
            clipboard,
            keyboard: app,
            access: Access(granted),
            copy_wait: Duration::ZERO,
        }
    }

    #[test]
    fn selection_exposed_by_accessibility_is_read_without_copying() {
        let clipboard = FakeClipboard::holding(text("abc"));
        let app = FrontApp::selecting(&clipboard, Some(text("other")));

        let read = reader(Some("今天天氣很好"), &clipboard, &app, true).selected_text();

        assert_eq!(read, Ok(Some("今天天氣很好".to_string())));
        assert_eq!((app.copies.get(), clipboard.changes.get()), (0, 0));
    }

    fn vscode_copy(line: &str, from_empty_selection: &str) -> ClipboardSnapshot {
        let json = format!(
            r#"{{"version":1,"isFromEmptySelection":{from_empty_selection},"multicursorText":null,"mode":"rust"}}"#
        );
        ClipboardSnapshot {
            items: vec![vec![
                (TEXT.to_string(), line.as_bytes().to_vec()),
                (VSCODE_EDITOR_DATA.to_string(), json.into_bytes()),
            ]],
        }
    }

    #[test]
    fn empty_selection_exposed_by_accessibility_means_nothing_selected_without_copying() {
        let clipboard = FakeClipboard::holding(text("abc"));
        let app = FrontApp::selecting(&clipboard, Some(text("let x = 1;\n")));

        let read = reader(Some(""), &clipboard, &app, true).selected_text();

        assert_eq!(read, Ok(None));
        assert_eq!(
            (
                app.copies.get(),
                clipboard.changes.get(),
                clipboard.current()
            ),
            (0, 0, text("abc"))
        );
    }

    #[test]
    fn vscode_copy_of_the_line_under_the_cursor_counts_as_nothing_selected() {
        let clipboard = FakeClipboard::holding(text("abc"));
        let app = FrontApp::selecting(&clipboard, Some(vscode_copy("let x = 1;\n", "true")));

        let read = reader(None, &clipboard, &app, true).selected_text();

        assert_eq!(read, Ok(None));
        assert_eq!((app.copies.get(), clipboard.current()), (1, text("abc")));
    }

    #[test]
    fn vscode_copy_of_a_real_selection_is_read() {
        let clipboard = FakeClipboard::holding(text("abc"));
        let app = FrontApp::selecting(&clipboard, Some(vscode_copy("x = 1", "false")));

        let read = reader(None, &clipboard, &app, true).selected_text();

        assert_eq!(read, Ok(Some("x = 1".to_string())));
        assert_eq!(clipboard.current(), text("abc"));
    }

    #[test]
    fn unreadable_vscode_data_does_not_hide_the_copied_text() {
        let mut copied = text("x = 1");
        copied.items[0].push((VSCODE_EDITOR_DATA.to_string(), b"{not json".to_vec()));
        let clipboard = FakeClipboard::holding(text("abc"));
        let app = FrontApp::selecting(&clipboard, Some(copied));

        let read = reader(None, &clipboard, &app, true).selected_text();

        assert_eq!(read, Ok(Some("x = 1".to_string())));
    }

    #[test]
    fn copied_selection_is_read_and_the_earlier_clipboard_is_put_back() {
        let clipboard = FakeClipboard::holding(text("abc"));
        let app = FrontApp::selecting(&clipboard, Some(text("今天天氣很好")));

        let read = reader(None, &clipboard, &app, true).selected_text();

        assert_eq!(read, Ok(Some("今天天氣很好".to_string())));
        assert_eq!(clipboard.current(), text("abc"));
    }

    #[test]
    fn copy_is_used_when_accessibility_exposes_nothing() {
        let clipboard = FakeClipboard::holding(image());
        let app = FrontApp::selecting(&clipboard, Some(text("Hello world")));

        let read = reader(None, &clipboard, &app, true).selected_text();

        assert_eq!(read, Ok(Some("Hello world".to_string())));
        assert_eq!((app.copies.get(), clipboard.current()), (1, image()));
    }

    #[test]
    fn empty_clipboard_is_left_empty_after_copying() {
        let clipboard = FakeClipboard::default();
        let app = FrontApp::selecting(&clipboard, Some(text("Hello world")));

        let read = reader(None, &clipboard, &app, true).selected_text();

        assert_eq!(read, Ok(Some("Hello world".to_string())));
        assert_eq!(clipboard.current(), ClipboardSnapshot::default());
    }

    #[test]
    fn nothing_copied_means_nothing_selected_and_the_clipboard_is_untouched() {
        let clipboard = FakeClipboard::holding(text("abc"));
        let app = FrontApp::selecting(&clipboard, None);

        let read = reader(None, &clipboard, &app, true).selected_text();

        assert_eq!(read, Ok(None));
        assert_eq!(
            (
                app.copies.get(),
                clipboard.changes.get(),
                clipboard.current()
            ),
            (1, 0, text("abc"))
        );
    }

    #[test]
    fn copied_content_without_text_counts_as_nothing_selected() {
        let clipboard = FakeClipboard::holding(text("abc"));
        let app = FrontApp::selecting(&clipboard, Some(image()));

        let read = reader(None, &clipboard, &app, true).selected_text();

        assert_eq!((read, clipboard.current()), (Ok(None), text("abc")));
    }

    #[test]
    fn missing_accessibility_permission_is_reported_without_copying() {
        let clipboard = FakeClipboard::holding(text("abc"));
        let app = FrontApp::selecting(&clipboard, Some(text("Hello world")));

        let read = reader(Some("Hello world"), &clipboard, &app, false).selected_text();

        assert_eq!(read, Err(Problem::AccessibilityPermission));
        assert_eq!((app.copies.get(), clipboard.current()), (0, text("abc")));
    }

    #[test]
    fn failing_to_send_copy_is_reported() {
        let clipboard = FakeClipboard::holding(text("abc"));
        let app = FrontApp {
            broken: true,
            ..FrontApp::selecting(&clipboard, Some(text("Hello world")))
        };

        let read = reader(None, &clipboard, &app, true).selected_text();

        assert!(matches!(read, Err(Problem::Other(_))), "{read:?}");
        assert_eq!(clipboard.current(), text("abc"));
    }

    #[test]
    fn failing_to_restore_the_clipboard_still_returns_the_selection() {
        let clipboard = FakeClipboard {
            restore_broken: true,
            ..FakeClipboard::holding(text("abc"))
        };
        let app = FrontApp::selecting(&clipboard, Some(text("Hello world")));

        let read = reader(None, &clipboard, &app, true).selected_text();

        assert_eq!(read, Ok(Some("Hello world".to_string())));
    }

    fn patient<'a>(
        clipboard: &'a FakeClipboard,
        app: &'a FrontApp<'a>,
    ) -> SelectionCopy<Focused, &'a FakeClipboard, &'a FrontApp<'a>, Access> {
        SelectionCopy {
            copy_wait: Duration::from_secs(2),
            ..reader(None, clipboard, app, true)
        }
    }

    #[test]
    fn text_written_after_the_app_cleared_the_clipboard_is_waited_for() {
        let clipboard = FakeClipboard::holding(text("abc"));
        // The app clears the clipboard at once and writes the text a few polls later.
        let app = FrontApp::selecting(&clipboard, Some(ClipboardSnapshot::default()));
        clipboard.schedule(4, text("今天天氣很好"));

        let read = patient(&clipboard, &app).selected_text();

        assert_eq!(read, Ok(Some("今天天氣很好".to_string())));
        assert_eq!(clipboard.current(), text("abc"));
    }

    #[test]
    fn a_cleared_clipboard_without_text_by_the_end_of_the_wait_is_nothing_selected() {
        let clipboard = FakeClipboard::holding(text("abc"));
        let app = FrontApp::selecting(&clipboard, Some(ClipboardSnapshot::default()));
        let reader = SelectionCopy {
            copy_wait: Duration::from_millis(30),
            ..reader(None, &clipboard, &app, true)
        };

        let read = reader.selected_text();

        assert_eq!((read, clipboard.current()), (Ok(None), text("abc")));
    }

    #[test]
    fn something_copied_after_our_copy_is_kept_on_the_clipboard() {
        let clipboard = FakeClipboard::holding(text("abc"));
        let app = FrontApp::selecting(&clipboard, Some(text("Hello world")));
        // Reads: before the copy, the poll that sees the copy, the check before restoring.
        clipboard.schedule(3, text("copied by the user meanwhile"));

        let read = patient(&clipboard, &app).selected_text();

        assert_eq!(read, Ok(Some("Hello world".to_string())));
        assert_eq!(clipboard.current(), text("copied by the user meanwhile"));
    }

    #[test]
    fn waiting_for_a_copy_gives_up_after_the_wait() {
        let clipboard = FakeClipboard::holding(text("abc"));
        let app = FrontApp::selecting(&clipboard, None);
        let reader = SelectionCopy {
            copy_wait: Duration::from_millis(30),
            ..reader(None, &clipboard, &app, true)
        };
        let started = Instant::now();

        let read = reader.selected_text();

        assert_eq!(read, Ok(None));
        assert!(started.elapsed() >= Duration::from_millis(30));
    }

    /// Asks the real Accessibility API; whatever it answers must not crash.
    #[test]
    fn system_focused_selection_can_be_queried() {
        let _ = SystemFocusedSelection.selected_text();
    }
}
