//! Session state machine: shortcut and clock events in, recorder commands out.
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

/// One dictation at a time; enforces minimum (inclusive) and maximum length and Escape.
#[derive(Debug, Default)]
pub struct SessionController {
    // implementation-defined state
}

impl SessionController {
    pub fn new() -> Self {
        todo!()
    }

    /// Feed one event observed at `now`; returns the command to execute, if any.
    pub fn handle(&mut self, event: ControllerEvent, now: Instant) -> Option<Command> {
        todo!()
    }
}
