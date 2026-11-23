//! View-model for the floating indicator and the menu-bar icon: what a `Status`
//! looks like, and for how long.
use eframe::egui::{Pos2, Vec2};
use std::time::{Duration, Instant};
use voice_core::{Action, Status};

/// How long Done and Cancelled stay visible.
pub const SHORT_MESSAGE: Duration = Duration::from_secs(2);
/// How long a Problem stays visible.
pub const PROBLEM_MESSAGE: Duration = Duration::from_secs(5);
/// Gap between the indicator and the bottom edge of the screen.
pub const BOTTOM_MARGIN: f32 = 96.0;
/// Size of the indicator window.
pub const INDICATOR_SIZE: Vec2 = Vec2::new(420.0, 48.0);

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
    /// Message; empty while listening (the chip and the timer say it all).
    pub text: String,
    /// Live input level in [0.0, 1.0]; 0.0 unless listening.
    pub level: f32,
    /// The action shown on the chip while listening.
    pub action: Option<Action>,
    /// Elapsed listening time as `m:ss`; only while listening.
    pub elapsed: Option<String>,
}

/// Elapsed time as `m:ss` (seconds rounded down).
pub fn format_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// What the indicator says while an action is being worked on.
pub fn working_text(action: Action) -> &'static str {
    match action {
        Action::Dictate => "Dictating…",
        Action::DictateInEnglish => "Translating…",
        Action::EditSelection => "Editing…",
    }
}

/// How long `status` stays on screen; `None` = until the status changes.
pub fn display_duration(status: &Status) -> Option<Duration> {
    match status {
        Status::Done | Status::Cancelled => Some(SHORT_MESSAGE),
        Status::Problem(_) => Some(PROBLEM_MESSAGE),
        Status::Idle | Status::Listening { .. } | Status::Working(_) => None,
    }
}

/// What the indicator shows for `status`; `None` = hidden. `listening_for` is how long
/// the current action has been listening.
pub fn indicator_view(status: &Status, listening_for: Duration) -> Option<IndicatorView> {
    let plain = |kind, text: String| IndicatorView {
        kind,
        text,
        level: 0.0,
        action: None,
        elapsed: None,
    };
    Some(match status {
        Status::Idle => return None,
        Status::Listening { action, level } => IndicatorView {
            kind: IndicatorKind::Listening,
            text: String::new(),
            level: clamp_level(*level),
            action: Some(*action),
            elapsed: Some(format_elapsed(listening_for)),
        },
        Status::Working(action) => plain(IndicatorKind::Working, working_text(*action).into()),
        Status::Done => plain(IndicatorKind::Done, "Done".into()),
        Status::Cancelled => plain(IndicatorKind::Cancelled, "Cancelled".into()),
        Status::Problem(problem) => plain(IndicatorKind::Problem, problem.to_string()),
    })
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
    /// When the current listening began; `None` unless listening.
    listening_since: Option<Instant>,
}

impl IndicatorState {
    pub fn new(now: Instant) -> Self {
        Self {
            status: Status::Idle,
            since: now,
            listening_since: None,
        }
    }

    /// Records `status`, observed at `now`. Repeating the current status does not
    /// restart its timer; every dictation starts its own listening timer at zero.
    pub fn set_status(&mut self, status: Status, now: Instant) {
        let listening = |s: &Status| match s {
            Status::Listening { action, .. } => Some(*action),
            _ => None,
        };
        match (listening(&self.status), listening(&status)) {
            (_, None) => self.listening_since = None,
            (previous, Some(action)) if previous != Some(action) => {
                self.listening_since = Some(now)
            }
            _ => {}
        }
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
        let listening_for = self
            .listening_since
            .map_or(Duration::ZERO, |t| now.saturating_duration_since(t));
        indicator_view(&self.effective_status(now), listening_for)
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

    fn listening(action: Action, level: f32) -> Status {
        Status::Listening { action, level }
    }

    fn view_of(status: &Status) -> IndicatorView {
        indicator_view(status, Duration::ZERO).unwrap()
    }

    #[test]
    fn idle_is_hidden() {
        assert_eq!(indicator_view(&Status::Idle, Duration::ZERO), None);
    }

    #[test]
    fn listening_shows_the_action_chip_and_the_live_level() {
        for action in Action::ALL {
            let view = view_of(&listening(action, 0.4));
            assert_eq!(view.kind, IndicatorKind::Listening);
            assert_eq!(view.action, Some(action));
            assert_eq!(view.level, 0.4);
            assert_eq!(view.text, "");
        }
    }

    #[test]
    fn the_chip_names_the_action() {
        let chip = |a: Action| view_of(&listening(a, 0.0)).action.unwrap().label();
        assert_eq!(chip(Action::Dictate), "Dictate");
        assert_eq!(chip(Action::DictateInEnglish), "English");
        assert_eq!(chip(Action::EditSelection), "Edit");
    }

    #[test]
    fn level_is_clamped_and_nan_is_silence() {
        let level = |l| view_of(&listening(Action::Dictate, l)).level;
        assert_eq!(level(7.0), 1.0);
        assert_eq!(level(-1.0), 0.0);
        assert_eq!(level(f32::NAN), 0.0);
    }

    #[test]
    fn elapsed_time_reads_as_minutes_and_seconds() {
        assert_eq!(format_elapsed(Duration::ZERO), "0:00");
        assert_eq!(format_elapsed(Duration::from_millis(999)), "0:00");
        assert_eq!(format_elapsed(secs(7)), "0:07");
        assert_eq!(format_elapsed(secs(59)), "0:59");
        assert_eq!(format_elapsed(secs(60)), "1:00");
        assert_eq!(format_elapsed(secs(83)), "1:23");
        assert_eq!(format_elapsed(secs(300)), "5:00");
    }

    #[test]
    fn working_text_names_what_is_being_done() {
        let text = |a| view_of(&Status::Working(a)).text;
        assert_eq!(text(Action::Dictate), "Dictating…");
        assert_eq!(text(Action::DictateInEnglish), "Translating…");
        assert_eq!(text(Action::EditSelection), "Editing…");
        let view = view_of(&Status::Working(Action::Dictate));
        assert_eq!(view.kind, IndicatorKind::Working);
        assert_eq!(view.level, 0.0);
        assert_eq!(view.elapsed, None);
        assert_eq!(view.action, None);
    }

    #[test]
    fn done_and_cancelled_have_their_own_text() {
        assert_eq!(view_of(&Status::Done).kind, IndicatorKind::Done);
        assert_eq!(view_of(&Status::Done).text, "Done");
        assert_eq!(view_of(&Status::Cancelled).kind, IndicatorKind::Cancelled);
        assert_eq!(view_of(&Status::Cancelled).text, "Cancelled");
    }

    #[test]
    fn problem_uses_its_display_text() {
        let view = view_of(&Status::Problem(Problem::NoSpeech));
        assert_eq!(view.kind, IndicatorKind::Problem);
        assert_eq!(view.text, "No speech heard");
        assert_eq!(view.level, 0.0);
        let unreachable = view_of(&Status::Problem(Problem::SpeechUnreachable(
            voice_core::SpeechProvider::OpenAi,
        )));
        assert_eq!(
            unreachable.text,
            "OpenAI not reachable — check your internet connection"
        );
    }

    #[test]
    fn display_durations() {
        assert_eq!(display_duration(&Status::Idle), None);
        assert_eq!(display_duration(&listening(Action::Dictate, 0.0)), None);
        assert_eq!(display_duration(&Status::Working(Action::Dictate)), None);
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
        state.set_status(Status::Working(Action::Dictate), t0);
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

    #[test]
    fn the_timer_counts_from_when_listening_began() {
        let t0 = Instant::now();
        let mut state = IndicatorState::new(t0);
        state.set_status(listening(Action::Dictate, 0.1), t0 + secs(10));
        state.set_status(listening(Action::Dictate, 0.7), t0 + secs(12));
        let elapsed = |at| state.view(at).unwrap().elapsed.unwrap();
        assert_eq!(elapsed(t0 + secs(10)), "0:00");
        assert_eq!(elapsed(t0 + secs(13)), "0:03");
        assert_eq!(elapsed(t0 + secs(75)), "1:05");
    }

    #[test]
    fn the_timer_resets_for_every_dictation() {
        let t0 = Instant::now();
        let mut state = IndicatorState::new(t0);
        state.set_status(listening(Action::Dictate, 0.2), t0);
        state.set_status(Status::Working(Action::Dictate), t0 + secs(8));
        state.set_status(Status::Done, t0 + secs(9));
        state.set_status(listening(Action::DictateInEnglish, 0.2), t0 + secs(20));
        assert_eq!(
            state.view(t0 + secs(22)).unwrap().elapsed.as_deref(),
            Some("0:02")
        );
    }

    #[test]
    fn a_new_dictation_right_after_idle_starts_at_zero() {
        let t0 = Instant::now();
        let mut state = IndicatorState::new(t0);
        state.set_status(listening(Action::EditSelection, 0.2), t0);
        state.set_status(Status::Idle, t0 + secs(30));
        state.set_status(listening(Action::EditSelection, 0.2), t0 + secs(40));
        assert_eq!(
            state.view(t0 + secs(40)).unwrap().elapsed.as_deref(),
            Some("0:00")
        );
    }

    #[test]
    fn only_listening_shows_a_timer() {
        let t0 = Instant::now();
        let mut state = IndicatorState::new(t0);
        state.set_status(listening(Action::Dictate, 0.2), t0);
        state.set_status(Status::Working(Action::Dictate), t0 + secs(3));
        assert_eq!(state.view(t0 + secs(4)).unwrap().elapsed, None);
    }
}
