//! Moving keyboard focus between this app and the app the user works in.
//!
//! The app lives in the menu bar only, so macOS never makes it the active app on
//! its own: a window it opens stays behind the windows of whichever app was active
//! before. Activating the app first lets its windows come to the front and take
//! keyboard focus. When its windows close, focus has to be handed back by hand too,
//! or the next dictation would paste into this app.
//!
//! Focus is handed back by activating the app that was in front before, never by
//! hiding this app: a hidden app gets no more frames from the UI toolkit, so the
//! menu-bar commands would stop working.
//!
//! AppKit may only be used from the main thread (where the UI runs). Called from
//! any other thread these functions do nothing and log a warning.
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSRunningApplication, NSWorkspace,
};
use std::cell::RefCell;

thread_local! {
    /// The app to give focus back to. Only ever touched on the main thread.
    static PREVIOUS_APP: RefCell<PreviousApp<Retained<NSRunningApplication>>> =
        const { RefCell::new(PreviousApp::none()) };
}

fn shared_app(purpose: &str) -> Option<Retained<NSApplication>> {
    match MainThreadMarker::new() {
        Some(mtm) => Some(NSApplication::sharedApplication(mtm)),
        None => {
            log::warn!("cannot {purpose} from a background thread");
            None
        }
    }
}

/// Makes this app the active app, even while another app is active, so its windows
/// can come to the front. The app that was active until now is remembered, so
/// `return_focus` can give focus back to it.
pub fn activate_app() {
    if let Some(app) = shared_app("bring the app to the front") {
        let front = NSWorkspace::sharedWorkspace().frontmostApplication();
        PREVIOUS_APP.with_borrow_mut(|previous| previous.remember(front));
        // The cooperative `activate` of macOS 14+ may be ignored here: the menu
        // command is handled on a later frame, outside the click that asked for it.
        // `activateIgnoringOtherApps` still works on every supported version.
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
}

/// Gives keyboard focus back to the app that was active before `activate_app`,
/// then forgets it. Does nothing when no app was remembered, when it has quit, or
/// when this app no longer has focus (the user has already moved on).
pub fn return_focus() {
    if let Some(app) = shared_app("give focus back") {
        let still_active = app.isActive();
        PREVIOUS_APP.with_borrow_mut(|previous| previous.hand_back(still_active));
    }
}

/// An app running on this Mac, as far as handing focus around is concerned.
trait RunningApp {
    fn is_this_app(&self) -> bool;
    fn has_quit(&self) -> bool;
    fn activate(&self);
}

impl RunningApp for Retained<NSRunningApplication> {
    fn is_this_app(&self) -> bool {
        u32::try_from(self.processIdentifier()).is_ok_and(|pid| pid == std::process::id())
    }

    fn has_quit(&self) -> bool {
        self.isTerminated()
    }

    fn activate(&self) {
        if !self.activateWithOptions(NSApplicationActivationOptions(0)) {
            log::warn!("could not give focus back to the previous app");
        }
    }
}

/// The app the user was in before this app took focus.
struct PreviousApp<A>(Option<A>);

impl<A: RunningApp> PreviousApp<A> {
    const fn none() -> Self {
        Self(None)
    }

    /// Remembers `front`, the app in front right now, unless it is this app: then
    /// the one remembered before still is the one to go back to.
    fn remember(&mut self, front: Option<A>) {
        if let Some(app) = front.filter(|app| !app.is_this_app()) {
            self.0 = Some(app);
        }
    }

    /// Activates the remembered app, if it is still running and this app still has
    /// focus, and forgets it either way.
    fn hand_back(&mut self, this_app_active: bool) {
        if let Some(app) = self.0.take() {
            if this_app_active && !app.has_quit() {
                app.activate();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[derive(Clone, Default)]
    struct FakeApp {
        this_app: bool,
        quit: bool,
        activated: Rc<Cell<usize>>,
    }

    impl RunningApp for FakeApp {
        fn is_this_app(&self) -> bool {
            self.this_app
        }
        fn has_quit(&self) -> bool {
            self.quit
        }
        fn activate(&self) {
            self.activated.set(self.activated.get() + 1);
        }
    }

    #[test]
    fn hands_focus_back_to_the_app_that_was_in_front() {
        let editor = FakeApp::default();
        let mut previous = PreviousApp::none();
        previous.remember(Some(editor.clone()));
        previous.hand_back(true);
        assert_eq!(editor.activated.get(), 1);
    }

    #[test]
    fn hands_focus_back_only_once() {
        let editor = FakeApp::default();
        let mut previous = PreviousApp::none();
        previous.remember(Some(editor.clone()));
        previous.hand_back(true);
        previous.hand_back(true);
        assert_eq!(editor.activated.get(), 1);
    }

    #[test]
    fn this_app_in_front_keeps_the_app_remembered_before() {
        let editor = FakeApp::default();
        let this = FakeApp {
            this_app: true,
            ..FakeApp::default()
        };
        let mut previous = PreviousApp::none();
        previous.remember(Some(editor.clone()));
        previous.remember(Some(this.clone()));
        previous.hand_back(true);
        assert_eq!(editor.activated.get(), 1);
        assert_eq!(this.activated.get(), 0);
    }

    #[test]
    fn the_latest_other_app_in_front_wins() {
        let editor = FakeApp::default();
        let browser = FakeApp::default();
        let mut previous = PreviousApp::none();
        previous.remember(Some(editor.clone()));
        previous.remember(Some(browser.clone()));
        previous.hand_back(true);
        assert_eq!(editor.activated.get(), 0);
        assert_eq!(browser.activated.get(), 1);
    }

    #[test]
    fn nothing_remembered_means_nothing_to_hand_back() {
        let mut previous = PreviousApp::<FakeApp>::none();
        previous.remember(None);
        previous.hand_back(true);
        assert!(previous.0.is_none());
    }

    #[test]
    fn an_app_that_has_quit_is_not_activated_and_is_forgotten() {
        let editor = FakeApp {
            quit: true,
            ..FakeApp::default()
        };
        let mut previous = PreviousApp::none();
        previous.remember(Some(editor.clone()));
        previous.hand_back(true);
        assert_eq!(editor.activated.get(), 0);
        assert!(previous.0.is_none());
    }

    #[test]
    fn once_the_user_has_moved_on_focus_is_left_where_it_is() {
        let editor = FakeApp::default();
        let mut previous = PreviousApp::none();
        previous.remember(Some(editor.clone()));
        previous.hand_back(false);
        assert_eq!(editor.activated.get(), 0);
        assert!(previous.0.is_none());
    }

    #[test]
    fn off_the_main_thread_nothing_is_done() {
        // Test threads are never the main thread, so AppKit must not be touched.
        std::thread::spawn(|| {
            activate_app();
            return_focus();
            PREVIOUS_APP.with_borrow(|previous| assert!(previous.0.is_none()));
        })
        .join()
        .unwrap();
    }
}
