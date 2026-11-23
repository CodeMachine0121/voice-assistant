//! The system facts the shortcut interpreter checks before it accepts a tap.
use crate::keys::Surroundings;
use objc2::rc::autoreleasepool;
use objc2_app_kit::NSWorkspace;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

/// Asks macOS directly. Both answers are cheap and safe from any thread.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SystemSurroundings;

impl Surroundings for SystemSurroundings {
    fn secure_input(&self) -> bool {
        // SAFETY: plain query without arguments.
        unsafe { IsSecureEventInputEnabled() != 0 }
    }

    fn frontmost_app(&self) -> Option<i32> {
        autoreleasepool(|_| {
            NSWorkspace::sharedWorkspace()
                .frontmostApplication()
                .map(|app| app.processIdentifier())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Talks to the real system; the answers depend on the machine, so only
    /// their shape is checked.
    #[test]
    fn system_surroundings_can_be_asked() {
        let around = SystemSurroundings;

        let _ = around.secure_input();
        let app = around.frontmost_app();

        assert!(app.is_none_or(|pid| pid > 0), "{app:?}");
    }
}
