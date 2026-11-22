//! View-model for the floating indicator and the menu-bar icon: what a `Status`
//! looks like, and for how long.
use eframe::egui::{Pos2, Vec2};
use std::time::{Duration, Instant};
use voice_core::Status;

/// How long Done and Cancelled stay visible.
pub const SHORT_MESSAGE: Duration = Duration::from_secs(2);
/// How long a Problem stays visible.
pub const PROBLEM_MESSAGE: Duration = Duration::from_secs(5);
/// Gap between the indicator and the bottom edge of the screen.
pub const BOTTOM_MARGIN: f32 = 96.0;
/// Size of the indicator window.
pub const INDICATOR_SIZE: Vec2 = Vec2::new(360.0, 48.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorKind {
    Listening,
    Working,
    Done,
    Cancelled,
    Problem,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorView {
    pub kind: IndicatorKind,
    pub text: String,
    /// Live input level in [0.0, 1.0]; 0.0 unless listening.
    pub level: f32,
}

/// How long `status` stays on screen; `None` = until the status changes.
pub fn display_duration(status: &Status) -> Option<Duration> {
    match status {
        Status::Done | Status::Cancelled => Some(SHORT_MESSAGE),
        Status::Problem(_) => Some(PROBLEM_MESSAGE),
        Status::Idle | Status::Listening { .. } | Status::Working => None,
    }
}

/// What the indicator shows for `status`; `None` = hidden.
pub fn indicator_view(status: &Status) -> Option<IndicatorView> {
    let (kind, text, level) = match status {
        Status::Idle => return None,
        Status::Listening { level } => (
            IndicatorKind::Listening,
            "Listening…".to_string(),
            clamp_level(*level),
        ),
        Status::Working => (IndicatorKind::Working, "Working…".to_string(), 0.0),
        Status::Done => (IndicatorKind::Done, "Done".to_string(), 0.0),
        Status::Cancelled => (IndicatorKind::Cancelled, "Cancelled".to_string(), 0.0),
        Status::Problem(problem) => (IndicatorKind::Problem, problem.to_string(), 0.0),
    };
    Some(IndicatorView { kind, text, level })
}

fn clamp_level(level: f32) -> f32 {
    if level.is_nan() {
        0.0
    } else {
        level.clamp(0.0, 1.0)
    }
}

/// Top-left corner that centres a window of `size` horizontally on a monitor of
/// `monitor` size, `BOTTOM_MARGIN` above its bottom edge.
pub fn indicator_position(monitor: Vec2, size: Vec2) -> Pos2 {
    Pos2::new(
        ((monitor.x - size.x) / 2.0).max(0.0),
        (monitor.y - size.y - BOTTOM_MARGIN).max(0.0),
    )
}

/// Tracks the latest status and when it arrived, so timed messages expire.
#[derive(Debug, Clone)]
pub struct IndicatorState {
    status: Status,
    since: Instant,
}

impl IndicatorState {
    pub fn new(now: Instant) -> Self {
        Self {
            status: Status::Idle,
            since: now,
        }
    }

    /// Records `status`, observed at `now`. Repeating the current status does not
    /// restart its timer.
    pub fn set_status(&mut self, status: Status, now: Instant) {
        if status != self.status {
            self.status = status;
            self.since = now;
        }
    }

    /// The status as the user should see it at `now`: timed messages that ran out
    /// read as Idle.
    pub fn effective_status(&self, now: Instant) -> Status {
        match display_duration(&self.status) {
            Some(limit) if now.saturating_duration_since(self.since) >= limit => Status::Idle,
            _ => self.status.clone(),
        }
    }

    pub fn view(&self, now: Instant) -> Option<IndicatorView> {
        indicator_view(&self.effective_status(now))
    }

    /// Time until the visible message expires; `None` when nothing is pending.
    pub fn time_to_expiry(&self, now: Instant) -> Option<Duration> {
        let limit = display_duration(&self.status)?;
        let elapsed = now.saturating_duration_since(self.since);
        (elapsed < limit).then(|| limit - elapsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voice_core::Problem;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn idle_is_hidden() {
        assert_eq!(indicator_view(&Status::Idle), None);
    }

    #[test]
    fn listening_shows_the_live_level() {
        let view = indicator_view(&Status::Listening { level: 0.4 }).unwrap();
        assert_eq!(view.kind, IndicatorKind::Listening);
        assert_eq!(view.text, "Listening…");
        assert_eq!(view.level, 0.4);
    }

    #[test]
    fn level_is_clamped_and_nan_is_silence() {
        let level = |l| {
            indicator_view(&Status::Listening { level: l })
                .unwrap()
                .level
        };
        assert_eq!(level(7.0), 1.0);
        assert_eq!(level(-1.0), 0.0);
        assert_eq!(level(f32::NAN), 0.0);
    }

    #[test]
    fn working_done_and_cancelled_have_their_own_text() {
        let view = |s| indicator_view(&s).unwrap();
        assert_eq!(view(Status::Working).kind, IndicatorKind::Working);
        assert_eq!(view(Status::Working).text, "Working…");
        assert_eq!(view(Status::Working).level, 0.0);
        assert_eq!(view(Status::Done).kind, IndicatorKind::Done);
        assert_eq!(view(Status::Done).text, "Done");
        assert_eq!(view(Status::Cancelled).kind, IndicatorKind::Cancelled);
        assert_eq!(view(Status::Cancelled).text, "Cancelled");
    }

    #[test]
    fn problem_uses_its_display_text() {
        let view = indicator_view(&Status::Problem(Problem::NoSpeech)).unwrap();
        assert_eq!(view.kind, IndicatorKind::Problem);
        assert_eq!(view.text, "No speech heard");
        assert_eq!(view.level, 0.0);
    }

    #[test]
    fn display_durations() {
        assert_eq!(display_duration(&Status::Idle), None);
        assert_eq!(display_duration(&Status::Listening { level: 0.0 }), None);
        assert_eq!(display_duration(&Status::Working), None);
        assert_eq!(display_duration(&Status::Done), Some(secs(2)));
        assert_eq!(display_duration(&Status::Cancelled), Some(secs(2)));
        assert_eq!(
            display_duration(&Status::Problem(Problem::NoSpeech)),
            Some(secs(5))
        );
    }

    #[test]
    fn position_is_bottom_center() {
        let pos = indicator_position(Vec2::new(1440.0, 900.0), Vec2::new(360.0, 48.0));
        assert_eq!(pos, Pos2::new(540.0, 900.0 - 48.0 - BOTTOM_MARGIN));
    }

    #[test]
    fn position_never_leaves_a_tiny_screen() {
        let pos = indicator_position(Vec2::new(100.0, 50.0), Vec2::new(360.0, 48.0));
        assert_eq!(pos, Pos2::new(0.0, 0.0));
    }

    #[test]
    fn done_expires_after_two_seconds() {
        let t0 = Instant::now();
        let mut state = IndicatorState::new(t0);
        state.set_status(Status::Done, t0);
        assert_eq!(
            state.view(t0 + Duration::from_millis(1999)).unwrap().text,
            "Done"
        );
        assert_eq!(state.view(t0 + secs(2)), None);
        assert_eq!(state.effective_status(t0 + secs(2)), Status::Idle);
    }

    #[test]
    fn problem_outlives_a_short_message() {
        let t0 = Instant::now();
        let mut state = IndicatorState::new(t0);
        state.set_status(Status::Problem(Problem::NoSpeech), t0);
        assert!(state.view(t0 + secs(4)).is_some());
        assert_eq!(state.view(t0 + secs(5)), None);
    }

    #[test]
    fn listening_and_working_never_expire() {
        let t0 = Instant::now();
        let mut state = IndicatorState::new(t0);
        state.set_status(Status::Working, t0);
        assert!(state.view(t0 + secs(3600)).is_some());
        assert_eq!(state.time_to_expiry(t0 + secs(3600)), None);
    }

    #[test]
    fn repeating_a_status_does_not_restart_its_timer() {
        let t0 = Instant::now();
        let mut state = IndicatorState::new(t0);
        state.set_status(Status::Done, t0);
        state.set_status(Status::Done, t0 + Duration::from_millis(1500));
        assert_eq!(state.view(t0 + secs(2)), None);
    }

    #[test]
    fn a_new_status_restarts_the_timer() {
        let t0 = Instant::now();
        let mut state = IndicatorState::new(t0);
        state.set_status(Status::Done, t0);
        state.set_status(Status::Cancelled, t0 + secs(1));
        assert!(state.view(t0 + Duration::from_millis(2500)).is_some());
        assert_eq!(state.view(t0 + secs(3)), None);
    }

    #[test]
    fn time_to_expiry_counts_down() {
        let t0 = Instant::now();
        let mut state = IndicatorState::new(t0);
        assert_eq!(state.time_to_expiry(t0), None);
        state.set_status(Status::Done, t0);
        assert_eq!(
            state.time_to_expiry(t0 + Duration::from_millis(500)),
            Some(Duration::from_millis(1500))
        );
        assert_eq!(state.time_to_expiry(t0 + secs(2)), None);
    }

    #[test]
    fn a_clock_running_backwards_does_not_panic() {
        let t0 = Instant::now();
        let mut state = IndicatorState::new(t0 + secs(10));
        state.set_status(Status::Done, t0 + secs(10));
        assert!(state.view(t0).is_some());
    }
}
