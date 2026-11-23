//! Moving keyboard focus between this app and the app the user works in.
//!
//! The app lives in the menu bar only, so macOS never makes it the active app on
//! its own: a window it opens stays behind the windows of whichever app was active
//! before. Activating the app first lets its windows come to the front and take
//! keyboard focus. When its windows close, focus has to be handed back by hand too,
//! or the next dictation would paste into this app.
//!
//! Which app focus goes back to is followed all the time: every time another app
//! becomes active, macOS says so and that app is remembered. Focus is handed back by
//! activating it, never by hiding this app: a hidden app gets no more frames from the
//! UI toolkit, so the menu-bar commands would stop working.
//!
//! AppKit may only be used from the main thread (where the UI runs). Called from
//! any other thread these functions do nothing and log a warning.
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSRunningApplication, NSWorkspace,
    NSWorkspaceApplicationKey, NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::{NSNotification, NSOperationQueue};
use std::cell::{Cell, RefCell};
use std::ptr::NonNull;

thread_local! {
    /// The app to give focus back to. Only ever touched on the main thread.
    static LAST_OTHER_APP: RefCell<LastOtherApp<Retained<NSRunningApplication>>> =
        const { RefCell::new(LastOtherApp::none()) };
    /// Whether macOS already tells us about every app that becomes active.
    static TRACKING: Cell<bool> = const { Cell::new(false) };
}

fn main_thread(purpose: &str) -> Option<MainThreadMarker> {
    let mtm = MainThreadMarker::new();
    if mtm.is_none() {
        log::warn!("cannot {purpose} from a background thread");
    }
    mtm
}

/// Starts following which other app the user was last in, so focus can be handed
/// back to it. Call once at startup on the main thread; later calls do nothing.
pub fn track_focus() {
    if main_thread("follow the app in front").is_none() || TRACKING.get() {
        return;
    }
    TRACKING.set(true);
    let workspace = NSWorkspace::sharedWorkspace();
    LAST_OTHER_APP.with_borrow_mut(|last| last.saw_in_front(workspace.frontmostApplication()));

    let block = RcBlock::new(|note: NonNull<NSNotification>| {
        // SAFETY: macOS hands the block a valid notification for its duration.
        let note = unsafe { note.as_ref() };
        let app = note
            .userInfo()
            // SAFETY: an AppKit constant, valid for the life of the process.
            .and_then(|info| info.objectForKey(unsafe { NSWorkspaceApplicationKey }))
            .and_then(|app| app.downcast::<NSRunningApplication>().ok());
        LAST_OTHER_APP.with_borrow_mut(|last| last.saw_in_front(app));
    });
    // SAFETY: the name is an AppKit constant; the block runs on the main queue,
    // so it only ever touches the main thread's state.
    let observer = unsafe {
        workspace
            .notificationCenter()
            .addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceDidActivateApplicationNotification),
                None,
                Some(&NSOperationQueue::mainQueue()),
                &block,
            )
    };
    // Observed for as long as the app runs.
    std::mem::forget(observer);
}

/// Makes this app the active app, even while another app is active, so its windows
/// can come to the front.
pub fn activate_app() {
    let Some(mtm) = main_thread("bring the app to the front") else {
        return;
    };
    // Covers the case where following the app in front has not started.
    let front = NSWorkspace::sharedWorkspace().frontmostApplication();
    LAST_OTHER_APP.with_borrow_mut(|last| last.saw_in_front(front));
    // The cooperative `activate` of macOS 14+ may be ignored here: the menu command
    // is handled on a later frame, outside the click that asked for it.
    // `activateIgnoringOtherApps` still works on every supported version.
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
}

/// Gives keyboard focus back to the other app the user was last in. When there is
/// none left, this app still gives up focus (without hiding), so the next dictation
/// never pastes into it. Does nothing when this app no longer has focus: the user
/// has already moved on.
pub fn return_focus() {
    let Some(mtm) = main_thread("give focus back") else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let decision = LAST_OTHER_APP.with_borrow_mut(|last| last.hand_back(app.isActive()));
    match decision {
        HandBack::Nothing => {}
        HandBack::Activate(other) if other.activate() => {}
        HandBack::Activate(_) | HandBack::GiveUp => app.deactivate(),
    }
}

/// An app running on this Mac, as far as handing focus around is concerned.
trait RunningApp {
    fn is_this_app(&self) -> bool;
    fn has_quit(&self) -> bool;
    /// `false` when macOS refused to activate it.
    fn activate(&self) -> bool;
}

impl RunningApp for Retained<NSRunningApplication> {
    fn is_this_app(&self) -> bool {
        u32::try_from(self.processIdentifier()).is_ok_and(|pid| pid == std::process::id())
    }

    fn has_quit(&self) -> bool {
        self.isTerminated()
    }

    fn activate(&self) -> bool {
        let done = self.activateWithOptions(NSApplicationActivationOptions(0));
        if !done {
            log::warn!("could not give focus back to the previous app");
        }
        done
    }
}

/// What to do with focus once the last window has closed.
#[derive(Debug, PartialEq)]
enum HandBack<A> {
    /// This app no longer has focus; leave it where it is.
    Nothing,
    /// Activate the other app the user was last in.
    Activate(A),
    /// No other app to go back to: this app just stops being the active one.
    GiveUp,
}

/// The other app the user was last in, that is the last app other than this one
/// that came to the front.
struct LastOtherApp<A>(Option<A>);

impl<A: RunningApp + Clone> LastOtherApp<A> {
    const fn none() -> Self {
        Self(None)
    }

    /// `front` is the app in front right now. This app is never one to go back to,
    /// so then the one remembered before stays.
    fn saw_in_front(&mut self, front: Option<A>) {
        if let Some(app) = front.filter(|app| !app.is_this_app()) {
            self.0 = Some(app);
        }
    }

    /// Decides where focus goes when this app is done. An app that has quit is
    /// forgotten.
    fn hand_back(&mut self, this_app_active: bool) -> HandBack<A> {
        if self.0.as_ref().is_some_and(A::has_quit) {
            self.0 = None;
        }
        match &self.0 {
            _ if !this_app_active => HandBack::Nothing,
            Some(app) => HandBack::Activate(app.clone()),
            None => HandBack::GiveUp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Default)]
    struct FakeApp {
        id: u8,
        this_app: bool,
        quit: bool,
    }

    const EDITOR: FakeApp = FakeApp {
        id: 1,
        this_app: false,
        quit: false,
    };
    const BROWSER: FakeApp = FakeApp {
        id: 2,
        this_app: false,
        quit: false,
    };
    const THIS: FakeApp = FakeApp {
        id: 3,
        this_app: true,
        quit: false,
    };

    impl RunningApp for FakeApp {
        fn is_this_app(&self) -> bool {
            self.this_app
        }
        fn has_quit(&self) -> bool {
            self.quit
        }
        fn activate(&self) -> bool {
            true
        }
    }

    fn after(seen: &[FakeApp]) -> LastOtherApp<FakeApp> {
        let mut last = LastOtherApp::none();
        for app in seen {
            last.saw_in_front(Some(*app));
        }
        last
    }

    #[test]
    fn focus_goes_back_to_the_app_the_user_was_in() {
        assert_eq!(
            after(&[EDITOR, THIS]).hand_back(true),
            HandBack::Activate(EDITOR)
        );
    }

    #[test]
    fn switching_apps_while_a_window_is_open_changes_where_focus_goes() {
        // Settings opened from the editor, then the user went to the browser and
        // came back to Settings.
        assert_eq!(
            after(&[EDITOR, THIS, BROWSER, THIS]).hand_back(true),
            HandBack::Activate(BROWSER)
        );
    }

    #[test]
    fn focus_goes_back_every_time_a_window_closes() {
        let mut last = after(&[EDITOR, THIS]);
        assert_eq!(last.hand_back(true), HandBack::Activate(EDITOR));
        last.saw_in_front(Some(EDITOR));
        last.saw_in_front(Some(THIS));
        assert_eq!(last.hand_back(true), HandBack::Activate(EDITOR));
    }

    #[test]
    fn without_another_app_this_app_gives_up_focus() {
        assert_eq!(after(&[THIS]).hand_back(true), HandBack::GiveUp);
        let mut nothing_seen = LastOtherApp::<FakeApp>::none();
        nothing_seen.saw_in_front(None);
        assert_eq!(nothing_seen.hand_back(true), HandBack::GiveUp);
    }

    #[test]
    fn an_app_that_has_quit_is_forgotten_and_this_app_gives_up_focus() {
        let quit_editor = FakeApp {
            quit: true,
            ..EDITOR
        };
        let mut last = after(&[quit_editor, THIS]);
        assert_eq!(last.hand_back(true), HandBack::GiveUp);
        assert!(last.0.is_none());
    }

    #[test]
    fn once_the_user_has_moved_on_focus_is_left_where_it_is() {
        assert_eq!(after(&[EDITOR, THIS]).hand_back(false), HandBack::Nothing);
        assert_eq!(after(&[]).hand_back(false), HandBack::Nothing);
    }

    #[test]
    fn off_the_main_thread_appkit_is_not_touched() {
        // Test threads are never the main thread.
        std::thread::spawn(|| {
            // The guard every AppKit entry point goes through refuses this thread...
            assert!(main_thread("test").is_none());
            // ...so these return before touching AppKit (which would abort here).
            track_focus();
            activate_app();
            return_focus();
        })
        .join()
        .unwrap();
    }
}
