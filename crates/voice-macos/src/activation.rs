//! Moving keyboard focus between this app and the app the user works in.
//!
//! The app lives in the menu bar only, so macOS never makes it the active app on
//! its own: a window it opens stays behind the windows of whichever app was active
//! before. Activating the app first lets its windows come to the front and take
//! keyboard focus. When its windows close, focus has to be handed back by hand too,
//! or the next dictation would paste into this app.
//!
//! AppKit may only be used from the main thread (where the UI runs). Called from
//! any other thread these functions do nothing and log a warning.
use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;

fn shared_app(purpose: &str) -> Option<objc2::rc::Retained<NSApplication>> {
    match MainThreadMarker::new() {
        Some(mtm) => Some(NSApplication::sharedApplication(mtm)),
        None => {
            log::warn!("cannot {purpose} from a background thread");
            None
        }
    }
}

/// Makes this app the active app, even while another app is active, so its windows
/// can come to the front.
pub fn activate_app() {
    if let Some(app) = shared_app("bring the app to the front") {
        // The cooperative `activate` of macOS 14+ may be ignored here: the menu
        // command is handled on a later frame, outside the click that asked for it.
        // `activateIgnoringOtherApps` still works on every supported version.
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
}

/// Hides this app, so macOS makes the app the user was in before active again.
pub fn hide_app() {
    if let Some(app) = shared_app("hide the app") {
        app.hide(None);
    }
}

/// Shows this app's windows again after `hide_app`, without taking keyboard focus
/// from the active app. Does nothing when the app is not hidden.
pub fn unhide_app_without_activation() {
    if let Some(app) = shared_app("show the app") {
        app.unhideWithoutActivation();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_the_main_thread_nothing_is_done() {
        // Test threads are never the main thread, so AppKit must not be touched.
        std::thread::spawn(|| {
            activate_app();
            hide_app();
            unhide_app_without_activation();
        })
        .join()
        .unwrap();
    }
}
