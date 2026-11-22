//! Session state machine: shortcut and clock events in, recorder commands out.
use crate::rules::{MAX_DICTATION, MIN_DICTATION};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutEvent {
    PushToTalkDown,
    PushToTalkUp,
    HandsFreePressed,
    EscapePressed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerEvent {
    Shortcut(ShortcutEvent),
    /// Periodic clock tick (enforces the maximum length).
    Tick,
    /// The pipeline finished the previous dictation; new ones may start.
    ProcessingFinished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Start capturing.
    StartRecording,
    /// Stop capturing and process what was heard.
    StopAndProcess,
    /// Stop capturing silently: the press was accidental (status back to Idle).
    Discard,
    /// Stop capturing because the user cancelled (status Cancelled).
    Cancel,
}

/// How the current dictation was started (and therefore how it is stopped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    PushToTalk,
    HandsFree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Listening { mode: Mode, since: Instant },
    Processing,
}

/// One dictation at a time; enforces minimum (inclusive) and maximum length and Escape.
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
        use ControllerEvent::{ProcessingFinished, Shortcut, Tick};
        use ShortcutEvent::{EscapePressed, HandsFreePressed, PushToTalkDown, PushToTalkUp};

        let (next, command) = match (self.state, event) {
            (State::Idle, Shortcut(PushToTalkDown)) => (
                Self::listening(Mode::PushToTalk, now),
                Some(Command::StartRecording),
            ),
            (State::Idle, Shortcut(HandsFreePressed)) => (
                Self::listening(Mode::HandsFree, now),
                Some(Command::StartRecording),
            ),
            (State::Listening { mode, since }, event) => match (mode, event) {
                (Mode::PushToTalk, Shortcut(PushToTalkUp))
                | (Mode::HandsFree, Shortcut(HandsFreePressed)) => Self::finish(since, now),
                (_, Shortcut(EscapePressed)) => (State::Idle, Some(Command::Cancel)),
                (_, Tick) if now.saturating_duration_since(since) >= MAX_DICTATION => {
                    (State::Processing, Some(Command::StopAndProcess))
                }
                _ => (self.state, None),
            },
            (State::Processing, ProcessingFinished) => (State::Idle, None),
            _ => (self.state, None),
        };
        self.state = next;
        command
    }

    fn listening(mode: Mode, since: Instant) -> State {
        State::Listening { mode, since }
    }

    /// The user ended the dictation: process it, or discard it when it was too short.
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
    use ControllerEvent::{ProcessingFinished, Shortcut, Tick};
    use ShortcutEvent::{EscapePressed, HandsFreePressed, PushToTalkDown, PushToTalkUp};

    const ALL_SHORTCUTS: [ShortcutEvent; 4] = [
        PushToTalkDown,
        PushToTalkUp,
        HandsFreePressed,
        EscapePressed,
    ];

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// A controller that started a push-to-talk dictation at `t0`.
    fn push_to_talk_started(t0: Instant) -> SessionController {
        let mut c = SessionController::new();
        assert_eq!(
            c.handle(Shortcut(PushToTalkDown), t0),
            Some(Command::StartRecording)
        );
        c
    }

    fn hands_free_started(t0: Instant) -> SessionController {
        let mut c = SessionController::new();
        assert_eq!(
            c.handle(Shortcut(HandsFreePressed), t0),
            Some(Command::StartRecording)
        );
        c
    }

    fn processing(t0: Instant) -> SessionController {
        let mut c = push_to_talk_started(t0);
        assert_eq!(
            c.handle(Shortcut(PushToTalkUp), t0 + ms(4000)),
            Some(Command::StopAndProcess)
        );
        c
    }

    fn assert_idle(c: &mut SessionController, now: Instant) {
        assert_eq!(
            c.handle(Shortcut(PushToTalkDown), now),
            Some(Command::StartRecording),
            "controller should be idle"
        );
    }

    #[test]
    fn holding_push_to_talk_starts_and_releasing_processes() {
        let t0 = Instant::now();
        let mut c = push_to_talk_started(t0);

        assert_eq!(
            c.handle(Shortcut(PushToTalkUp), t0 + ms(4000)),
            Some(Command::StopAndProcess)
        );
    }

    #[test]
    fn a_press_of_exactly_half_a_second_counts() {
        let t0 = Instant::now();
        let mut c = push_to_talk_started(t0);

        assert_eq!(
            c.handle(Shortcut(PushToTalkUp), t0 + ms(500)),
            Some(Command::StopAndProcess)
        );
    }

    #[test]
    fn a_very_short_press_is_discarded_and_returns_to_idle() {
        let t0 = Instant::now();
        let mut c = push_to_talk_started(t0);

        assert_eq!(
            c.handle(Shortcut(PushToTalkUp), t0 + ms(300)),
            Some(Command::Discard)
        );
        assert_idle(&mut c, t0 + ms(400));
    }

    #[test]
    fn a_press_just_under_half_a_second_is_discarded() {
        let t0 = Instant::now();
        let mut c = push_to_talk_started(t0);

        assert_eq!(
            c.handle(Shortcut(PushToTalkUp), t0 + ms(499)),
            Some(Command::Discard)
        );
    }

    #[test]
    fn escape_cancels_push_to_talk_and_returns_to_idle() {
        let t0 = Instant::now();
        let mut c = push_to_talk_started(t0);

        assert_eq!(
            c.handle(Shortcut(EscapePressed), t0 + ms(2000)),
            Some(Command::Cancel)
        );
        assert_eq!(c.handle(Shortcut(PushToTalkUp), t0 + ms(2100)), None);
        assert_idle(&mut c, t0 + ms(2200));
    }

    #[test]
    fn hands_free_press_starts_and_second_press_processes() {
        let t0 = Instant::now();
        let mut c = hands_free_started(t0);

        assert_eq!(
            c.handle(Shortcut(HandsFreePressed), t0 + ms(3000)),
            Some(Command::StopAndProcess)
        );
    }

    #[test]
    fn hands_free_stopped_too_soon_is_discarded() {
        let t0 = Instant::now();
        let mut c = hands_free_started(t0);

        assert_eq!(
            c.handle(Shortcut(HandsFreePressed), t0 + ms(200)),
            Some(Command::Discard)
        );
        assert_idle(&mut c, t0 + ms(300));
    }

    #[test]
    fn escape_cancels_hands_free() {
        let t0 = Instant::now();
        let mut c = hands_free_started(t0);

        assert_eq!(
            c.handle(Shortcut(EscapePressed), t0 + ms(60_000)),
            Some(Command::Cancel)
        );
        assert_idle(&mut c, t0 + ms(60_100));
    }

    #[test]
    fn hands_free_stops_on_its_own_at_five_minutes() {
        let t0 = Instant::now();
        let mut c = hands_free_started(t0);

        assert_eq!(c.handle(Tick, t0 + MAX_DICTATION - ms(1)), None);
        assert_eq!(
            c.handle(Tick, t0 + MAX_DICTATION),
            Some(Command::StopAndProcess)
        );
    }

    #[test]
    fn push_to_talk_also_stops_at_five_minutes_and_release_is_then_ignored() {
        let t0 = Instant::now();
        let mut c = push_to_talk_started(t0);

        assert_eq!(
            c.handle(Tick, t0 + MAX_DICTATION + ms(50)),
            Some(Command::StopAndProcess)
        );
        assert_eq!(
            c.handle(Shortcut(PushToTalkUp), t0 + MAX_DICTATION + ms(60)),
            None
        );
    }

    #[test]
    fn shortcuts_and_ticks_are_ignored_while_processing() {
        let t0 = Instant::now();
        let mut c = processing(t0);

        for (i, shortcut) in ALL_SHORTCUTS.into_iter().enumerate() {
            let now = t0 + ms(5000 + i as u64 * 1000);
            assert_eq!(c.handle(Shortcut(shortcut), now), None, "{shortcut:?}");
        }
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
    fn hands_free_can_start_after_processing_finished() {
        let t0 = Instant::now();
        let mut c = processing(t0);
        c.handle(ProcessingFinished, t0 + ms(5000));

        assert_eq!(
            c.handle(Shortcut(HandsFreePressed), t0 + ms(5100)),
            Some(Command::StartRecording)
        );
    }

    #[test]
    fn events_that_do_not_apply_while_idle_do_nothing() {
        let t0 = Instant::now();
        let mut c = SessionController::new();

        assert_eq!(c.handle(Shortcut(PushToTalkUp), t0), None);
        assert_eq!(c.handle(Shortcut(EscapePressed), t0), None);
        assert_eq!(c.handle(Tick, t0 + MAX_DICTATION), None);
        assert_eq!(c.handle(ProcessingFinished, t0), None);
        assert_idle(&mut c, t0);
    }

    #[test]
    fn hands_free_press_during_push_to_talk_is_ignored() {
        let t0 = Instant::now();
        let mut c = push_to_talk_started(t0);

        assert_eq!(c.handle(Shortcut(HandsFreePressed), t0 + ms(1000)), None);
        assert_eq!(c.handle(Shortcut(PushToTalkDown), t0 + ms(1100)), None);
        assert_eq!(c.handle(ProcessingFinished, t0 + ms(1150)), None);
        assert_eq!(
            c.handle(Shortcut(PushToTalkUp), t0 + ms(1200)),
            Some(Command::StopAndProcess)
        );
    }

    #[test]
    fn push_to_talk_keys_during_hands_free_are_ignored() {
        let t0 = Instant::now();
        let mut c = hands_free_started(t0);

        assert_eq!(c.handle(Shortcut(PushToTalkDown), t0 + ms(1000)), None);
        assert_eq!(c.handle(Shortcut(PushToTalkUp), t0 + ms(1100)), None);
        assert_eq!(
            c.handle(Shortcut(HandsFreePressed), t0 + ms(1200)),
            Some(Command::StopAndProcess)
        );
    }

    #[test]
    fn default_is_an_idle_controller() {
        let mut c = SessionController::default();

        assert_idle(&mut c, Instant::now());
    }
}
