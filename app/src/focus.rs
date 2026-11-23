//! Bringing a window to the front when the user asks for it.
//!
//! A window that was just opened is not shown until it has been drawn once, and
//! macOS ignores a request to focus a window that is not shown yet. So the request
//! is repeated on every frame until the window has keyboard focus, giving up after
//! a short while so the app never fights the user for focus.
use std::time::{Duration, Instant};

/// How long a window keeps asking for focus.
pub const FOCUS_PATIENCE: Duration = Duration::from_secs(2);

/// An outstanding wish to bring one window to the front.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FocusRequest {
    until: Option<Instant>,
}

impl FocusRequest {
    /// Starts (or restarts) asking for focus.
    pub fn request(&mut self, now: Instant) {
        self.until = Some(now + FOCUS_PATIENCE);
    }

    /// Called on every frame of the window with whether it has keyboard focus.
    /// `true` = ask for focus now. Ends once the window is focused or patience runs out.
    pub fn poll(&mut self, focused: bool, now: Instant) -> bool {
        match self.until {
            Some(until) if !focused && now < until => true,
            _ => {
                self.until = None;
                false
            }
        }
    }

    pub fn cancel(&mut self) {
        self.until = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_asked_without_a_request() {
        let mut focus = FocusRequest::default();
        assert!(!focus.poll(false, Instant::now()));
    }

    #[test]
    fn keeps_asking_until_the_window_is_focused() {
        let now = Instant::now();
        let mut focus = FocusRequest::default();
        focus.request(now);
        assert!(focus.poll(false, now));
        assert!(focus.poll(false, now + Duration::from_millis(100)));
        assert!(!focus.poll(true, now + Duration::from_millis(200)));
        assert!(!focus.poll(false, now + Duration::from_millis(300)));
    }

    #[test]
    fn gives_up_after_a_while() {
        let now = Instant::now();
        let mut focus = FocusRequest::default();
        focus.request(now);
        assert!(!focus.poll(false, now + FOCUS_PATIENCE));
        assert!(!focus.poll(false, now));
    }

    #[test]
    fn a_new_request_restarts_the_wait() {
        let now = Instant::now();
        let mut focus = FocusRequest::default();
        focus.request(now);
        let later = now + FOCUS_PATIENCE;
        focus.request(later);
        assert!(focus.poll(false, later + Duration::from_millis(1)));
    }

    #[test]
    fn cancelling_stops_asking() {
        let now = Instant::now();
        let mut focus = FocusRequest::default();
        focus.request(now);
        focus.cancel();
        assert!(!focus.poll(false, now));
    }
}
