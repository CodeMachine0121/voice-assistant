//! Session state machine: shortcut and clock events in, recorder commands out.
use crate::rules::{MAX_DICTATION, MIN_DICTATION};
use crate::Action;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutEvent {
    /// The shortcut of this action was tapped (starts it, or stops it when it is listening).
    ActionTapped(Action),
    EscapePressed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerEvent {
    Shortcut(ShortcutEvent),
    /// Periodic clock tick (enforces the maximum length).
    Tick,
    /// The pipeline finished the previous recording; new ones may start.
    ProcessingFinished,
    /// The action could not start (not ready, or the recorder failed): it never began.
    RecordingFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Get this action ready and start capturing.
    StartRecording(Action),
    /// Stop capturing and process what was heard.
    StopAndProcess,
    /// Stop capturing silently: the tap was accidental (status back to Idle).
    Discard,
    /// Stop capturing because the user cancelled (status Cancelled).
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Listening { action: Action, since: Instant },
    Processing,
}

/// One action at a time, started and stopped by its own shortcut; enforces minimum
/// (inclusive) and maximum length and Escape.
#[derive(Debug)]
pub struct SessionController {
    state: State,
}

impl Default for SessionController {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionController {
    pub fn new() -> Self {
        Self { state: State::Idle }
    }

    /// Feed one event observed at `now`; returns the command to execute, if any.
    pub fn handle(&mut self, event: ControllerEvent, now: Instant) -> Option<Command> {
        use ControllerEvent::{ProcessingFinished, RecordingFailed, Shortcut, Tick};
        use ShortcutEvent::{ActionTapped, EscapePressed};

        let (next, command) = match (self.state, event) {
            (State::Idle, Shortcut(ActionTapped(action))) => (
                State::Listening { action, since: now },
                Some(Command::StartRecording(action)),
            ),
            (State::Listening { action, since }, event) => match event {
                Shortcut(ActionTapped(tapped)) if tapped == action => Self::finish(since, now),
                Shortcut(EscapePressed) => (State::Idle, Some(Command::Cancel)),
                RecordingFailed => (State::Idle, None),
                Tick if now.saturating_duration_since(since) >= MAX_DICTATION => {
                    (State::Processing, Some(Command::StopAndProcess))
                }
                // Another action's shortcut, early ticks, stray completions: keep listening.
                _ => (self.state, None),
            },
            (State::Processing, ProcessingFinished) => (State::Idle, None),
            _ => (self.state, None),
        };
        self.state = next;
        command
    }

    /// Whether a recording is being captured right now.
    pub fn is_listening(&self) -> bool {
        matches!(self.state, State::Listening { .. })
    }

    /// The user ended the recording: process it, or discard it when it was too short.
    fn finish(since: Instant, now: Instant) -> (State, Option<Command>) {
        if now.saturating_duration_since(since) >= MIN_DICTATION {
            (State::Processing, Some(Command::StopAndProcess))
        } else {
            (State::Idle, Some(Command::Discard))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use Action::{Dictate, DictateInEnglish, EditSelection};
    use ControllerEvent::{ProcessingFinished, RecordingFailed, Shortcut, Tick};
    use ShortcutEvent::{ActionTapped, EscapePressed};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn tap(action: Action) -> ControllerEvent {
        Shortcut(ActionTapped(action))
    }

    /// A controller that started `action` at `t0`.
    fn started(action: Action, t0: Instant) -> SessionController {
        let mut c = SessionController::new();
        assert_eq!(
            c.handle(tap(action), t0),
            Some(Command::StartRecording(action))
        );
        c
    }

    fn processing(t0: Instant) -> SessionController {
        let mut c = started(Dictate, t0);
        assert_eq!(
            c.handle(tap(Dictate), t0 + ms(4000)),
            Some(Command::StopAndProcess)
        );
        c
    }

    fn assert_idle(c: &mut SessionController, now: Instant) {
        assert_eq!(
            c.handle(tap(Dictate), now),
            Some(Command::StartRecording(Dictate)),
            "controller should be idle"
        );
    }

    #[test]
    fn each_action_starts_with_its_own_shortcut_and_stops_with_it() {
        for action in Action::ALL {
            let t0 = Instant::now();
            let mut c = started(action, t0);

            assert_eq!(
                c.handle(tap(action), t0 + ms(3000)),
                Some(Command::StopAndProcess),
                "{action:?}"
            );
        }
    }

    #[test]
    fn another_actions_shortcut_is_ignored_while_one_is_listening() {
        let t0 = Instant::now();
        let mut c = started(Dictate, t0);

        assert_eq!(c.handle(tap(DictateInEnglish), t0 + ms(1000)), None);
        assert_eq!(c.handle(tap(EditSelection), t0 + ms(1100)), None);
        assert!(c.is_listening());
        assert_eq!(
            c.handle(tap(Dictate), t0 + ms(1200)),
            Some(Command::StopAndProcess)
        );
    }

    #[test]
    fn stopping_at_exactly_half_a_second_counts() {
        let t0 = Instant::now();
        let mut c = started(Dictate, t0);

        assert_eq!(
            c.handle(tap(Dictate), t0 + ms(500)),
            Some(Command::StopAndProcess)
        );
    }

    #[test]
    fn stopping_within_half_a_second_is_discarded_and_returns_to_idle() {
        let t0 = Instant::now();
        let mut c = started(Dictate, t0);

        assert_eq!(c.handle(tap(Dictate), t0 + ms(300)), Some(Command::Discard));
        assert_idle(&mut c, t0 + ms(400));
    }

    #[test]
    fn stopping_just_under_half_a_second_is_discarded() {
        let t0 = Instant::now();
        let mut c = started(EditSelection, t0);

        assert_eq!(
            c.handle(tap(EditSelection), t0 + ms(499)),
            Some(Command::Discard)
        );
    }

    #[test]
    fn escape_cancels_any_action_and_returns_to_idle() {
        for action in Action::ALL {
            let t0 = Instant::now();
            let mut c = started(action, t0);

            assert_eq!(
                c.handle(Shortcut(EscapePressed), t0 + ms(2000)),
                Some(Command::Cancel),
                "{action:?}"
            );
            assert_eq!(
                c.handle(tap(action), t0 + ms(2100)),
                Some(Command::StartRecording(action))
            );
        }
    }

    #[test]
    fn any_action_stops_on_its_own_at_five_minutes() {
        for action in Action::ALL {
            let t0 = Instant::now();
            let mut c = started(action, t0);

            assert_eq!(c.handle(Tick, t0 + MAX_DICTATION - ms(1)), None);
            assert_eq!(
                c.handle(Tick, t0 + MAX_DICTATION),
                Some(Command::StopAndProcess),
                "{action:?}"
            );
            // the user's late tap no longer does anything
            assert_eq!(c.handle(tap(action), t0 + MAX_DICTATION + ms(10)), None);
        }
    }

    #[test]
    fn shortcuts_and_ticks_are_ignored_while_processing() {
        let t0 = Instant::now();
        let mut c = processing(t0);

        for (i, action) in Action::ALL.into_iter().enumerate() {
            let now = t0 + ms(5000 + i as u64 * 1000);
            assert_eq!(c.handle(tap(action), now), None, "{action:?}");
        }
        assert_eq!(c.handle(Shortcut(EscapePressed), t0 + ms(9000)), None);
        assert_eq!(c.handle(Tick, t0 + MAX_DICTATION * 2), None);
    }

    #[test]
    fn processing_finished_returns_to_idle() {
        let t0 = Instant::now();
        let mut c = processing(t0);

        assert_eq!(c.handle(ProcessingFinished, t0 + ms(5000)), None);
        assert_idle(&mut c, t0 + ms(5100));
    }

    #[test]
    fn a_different_action_can_start_after_processing_finished() {
        let t0 = Instant::now();
        let mut c = processing(t0);
        c.handle(ProcessingFinished, t0 + ms(5000));

        assert_eq!(
            c.handle(tap(EditSelection), t0 + ms(5100)),
            Some(Command::StartRecording(EditSelection))
        );
    }

    #[test]
    fn events_that_do_not_apply_while_idle_do_nothing() {
        let t0 = Instant::now();
        let mut c = SessionController::new();

        assert_eq!(c.handle(Shortcut(EscapePressed), t0), None);
        assert_eq!(c.handle(Tick, t0 + MAX_DICTATION), None);
        assert_eq!(c.handle(ProcessingFinished, t0), None);
        assert_idle(&mut c, t0);
    }

    #[test]
    fn processing_finished_while_listening_is_ignored() {
        let t0 = Instant::now();
        let mut c = started(DictateInEnglish, t0);

        assert_eq!(c.handle(ProcessingFinished, t0 + ms(100)), None);
        assert!(c.is_listening());
    }

    #[test]
    fn default_is_an_idle_controller() {
        let mut c = SessionController::default();

        assert_idle(&mut c, Instant::now());
    }

    #[test]
    fn a_failed_start_returns_to_idle_without_a_command() {
        let t0 = Instant::now();
        let mut c = started(EditSelection, t0);

        assert_eq!(c.handle(RecordingFailed, t0), None);
        assert!(!c.is_listening());
        assert_eq!(c.handle(Tick, t0 + MAX_DICTATION), None);
        assert_eq!(
            c.handle(tap(EditSelection), t0 + ms(4000)),
            Some(Command::StartRecording(EditSelection)),
            "a new attempt is allowed"
        );
    }

    #[test]
    fn recording_failure_is_ignored_while_idle() {
        let t0 = Instant::now();
        let mut c = SessionController::new();

        assert_eq!(c.handle(RecordingFailed, t0), None);
        assert_idle(&mut c, t0);
    }

    #[test]
    fn recording_failure_is_ignored_while_processing() {
        let t0 = Instant::now();
        let mut c = processing(t0);

        assert_eq!(c.handle(RecordingFailed, t0 + ms(5000)), None);
        assert_eq!(c.handle(tap(Dictate), t0 + ms(5100)), None);
        assert_eq!(c.handle(ProcessingFinished, t0 + ms(5200)), None);
        assert_idle(&mut c, t0 + ms(5300));
    }

    #[test]
    fn is_listening_only_while_a_recording_is_captured() {
        let t0 = Instant::now();
        let mut c = SessionController::new();
        assert!(!c.is_listening(), "idle");

        c.handle(tap(DictateInEnglish), t0);
        assert!(c.is_listening(), "listening");

        c.handle(tap(DictateInEnglish), t0 + ms(3000));
        assert!(!c.is_listening(), "processing");

        c.handle(ProcessingFinished, t0 + ms(4000));
        c.handle(tap(EditSelection), t0 + ms(5000));
        assert!(c.is_listening(), "listening again");

        c.handle(Shortcut(EscapePressed), t0 + ms(6000));
        assert!(!c.is_listening(), "cancelled");
    }
}
