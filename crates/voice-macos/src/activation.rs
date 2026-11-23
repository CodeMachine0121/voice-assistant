//! Bringing the app to the front.
//!
//! The app lives in the menu bar only, so macOS never makes it the active app on
//! its own: a window it opens stays behind the windows of whichever app was active
//! before. Activating the app first lets its windows come to the front and take
//! keyboard focus.
use objc2::sel;
use objc2_app_kit::NSApplication;
use objc2_foundation::{MainThreadMarker, NSObjectProtocol};

/// Makes this app the active app, so its windows can come to the front.
///
/// AppKit may only be used from the main thread (where the UI runs). Called from
/// any other thread it does nothing and logs a warning.
pub fn activate_app() {
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("cannot bring the app to the front from a background thread");
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    if app.respondsToSelector(sel!(activate)) {
        // macOS 14 and later.
        app.activate();
    } else {
        // macOS 13 has no `activate` yet.
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_the_main_thread_it_does_nothing() {
        // Test threads are never the main thread, so AppKit must not be touched.
        std::thread::spawn(activate_app).join().unwrap();
    }
}
