//! Global shortcut listener: a `CGEventTap` on its own thread.
//!
//! The tap thread only decodes events, forwards them to [`KeyInterpreter`] and
//! drops the ones it says to consume (the Space of a combo shortcut); every
//! mapping rule lives there.
use crate::keys::{KeyDecision, KeyInput, KeyInterpreter, KeyKind, ShortcutConfig};
use core_foundation::base::TCFType;
use core_foundation::mach_port::CFMachPortRef;
use core_foundation::runloop::{kCFRunLoopCommonModes, kCFRunLoopDefaultMode, CFRunLoop};
use core_graphics::event::{
    CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult, EventField,
};
use crossbeam_channel::Sender;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;
use voice_core::{Problem, ShortcutEvent};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

/// Decodes the parts of a `CGEvent` the interpreter needs. `None` for event
/// types that are neither keyboard events nor mouse-button presses.
pub(crate) fn decode(
    event_type: CGEventType,
    keycode: i64,
    flags: u64,
    autorepeat: i64,
) -> Option<KeyInput> {
    let kind = match event_type {
        CGEventType::KeyDown => KeyKind::Down,
        CGEventType::KeyUp => KeyKind::Up,
        CGEventType::FlagsChanged => KeyKind::FlagsChanged,
        CGEventType::LeftMouseDown | CGEventType::RightMouseDown | CGEventType::OtherMouseDown => {
            KeyKind::Pointer
        }
        _ => return None,
    };
    Some(KeyInput {
        kind,
        keycode: u16::try_from(keycode).unwrap_or(u16::MAX),
        flags,
        autorepeat: autorepeat != 0,
    })
}

/// macOS turns a tap off when its callback is slow or on secure input.
fn is_tap_disabled(event_type: CGEventType) -> bool {
    matches!(
        event_type,
        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
    )
}

struct Shared {
    interpreter: Mutex<KeyInterpreter>,
    sink: Sender<ShortcutEvent>,
    stopped: AtomicBool,
}

/// Upper bound on how long the tap thread may miss a stop request.
const STOP_POLL: Duration = Duration::from_millis(500);

impl Shared {
    fn interpreter(&self) -> MutexGuard<'_, KeyInterpreter> {
        // The interpreter has no invariants a panic could break; keep going.
        self.interpreter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Interprets one input, forwards what it means and tells whether to drop it.
    fn handle(&self, input: Option<KeyInput>) -> bool {
        let Some(input) = input else {
            return false;
        };
        let KeyDecision { event, consume } = self.interpreter().on_key(input);
        if let Some(event) = event {
            // The receiver is gone only while the app shuts down.
            let _ = self.sink.send(event);
        }
        consume
    }
}

/// Starts the global shortcut listener.
pub struct EventTapShortcuts;

impl EventTapShortcuts {
    /// Fails with `Problem::AccessibilityPermission` when macOS refuses the tap
    /// (Input Monitoring / Accessibility not granted).
    pub fn start(
        config: ShortcutConfig,
        sink: Sender<ShortcutEvent>,
    ) -> Result<ShortcutListener, Problem> {
        let shared = Arc::new(Shared {
            interpreter: Mutex::new(KeyInterpreter::new(&config)),
            sink,
            stopped: AtomicBool::new(false),
        });
        let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
        let tap_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("shortcut-tap".into())
            .spawn(move || run_tap(&tap_shared, &ready_tx))
            .map_err(|e| Problem::Other(format!("Could not start shortcut listener: {e}")))?;
        match ready_rx.recv() {
            Ok(Some(run_loop)) => Ok(ShortcutListener {
                shared,
                run_loop,
                thread: Some(thread),
            }),
            _ => {
                let _ = thread.join();
                Err(Problem::AccessibilityPermission)
            }
        }
    }
}

/// Body of the tap thread: reports its run loop (or `None` on failure), then
/// runs until the listener stops it.
fn run_tap(shared: &Arc<Shared>, ready: &Sender<Option<CFRunLoop>>) {
    // Raw port of the tap, so the callback can re-enable it; set right after creation.
    let port: Arc<Mutex<Option<usize>>> = Arc::new(Mutex::new(None));
    let callback_port = Arc::clone(&port);
    let callback_shared = Arc::clone(shared);
    let callback = move |_proxy, event_type: CGEventType, event: &CGEvent| {
        if is_tap_disabled(event_type) {
            if let Some(raw) = *callback_port.lock().unwrap_or_else(PoisonError::into_inner) {
                // SAFETY: the port belongs to the tap, which outlives its callback.
                unsafe { CGEventTapEnable(raw as CFMachPortRef, true) };
            }
            return CallbackResult::Keep;
        }
        let consume = callback_shared.handle(decode(
            event_type,
            event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE),
            event.get_flags().bits(),
            event.get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT),
        ));
        if consume {
            CallbackResult::Drop
        } else {
            CallbackResult::Keep
        }
    };
    let tap = CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        // Not listen-only: the Space of a combo shortcut must be dropped.
        CGEventTapOptions::Default,
        vec![
            CGEventType::KeyDown,
            CGEventType::KeyUp,
            CGEventType::FlagsChanged,
            CGEventType::LeftMouseDown,
            CGEventType::RightMouseDown,
            CGEventType::OtherMouseDown,
        ],
        callback,
    );
    let Ok(tap) = tap else {
        let _ = ready.send(None);
        return;
    };
    let Ok(source) = tap.mach_port().create_runloop_source(0) else {
        let _ = ready.send(None);
        return;
    };
    *port.lock().unwrap_or_else(PoisonError::into_inner) =
        Some(tap.mach_port().as_concrete_TypeRef() as usize);
    let run_loop = CFRunLoop::get_current();
    // SAFETY: reading an immutable CoreFoundation constant.
    run_loop.add_source(&source, unsafe { kCFRunLoopCommonModes });
    tap.enable();
    let _ = ready.send(Some(run_loop));
    // A stop requested before the loop starts running would be lost, so the
    // flag is checked between bounded runs as well.
    while !shared.stopped.load(Ordering::Acquire) {
        // SAFETY: reading an immutable CoreFoundation constant.
        CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, STOP_POLL, false);
    }
    *port.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

/// A running shortcut listener. Dropping it stops the tap.
pub struct ShortcutListener {
    shared: Arc<Shared>,
    run_loop: CFRunLoop,
    thread: Option<JoinHandle<()>>,
}

impl ShortcutListener {
    /// Switches to new shortcuts without restarting the tap.
    pub fn update(&self, config: ShortcutConfig) {
        self.shared.interpreter().reconfigure(&config);
    }
}

impl Drop for ShortcutListener {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        self.run_loop.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voice_core::Action;

    #[test]
    fn keyboard_event_types_are_decoded() {
        let decoded = [
            CGEventType::KeyDown,
            CGEventType::KeyUp,
            CGEventType::FlagsChanged,
        ]
        .map(|t| decode(t, 61, 0x40, 1).map(|i| i.kind));

        assert_eq!(
            decoded,
            [
                Some(KeyKind::Down),
                Some(KeyKind::Up),
                Some(KeyKind::FlagsChanged)
            ]
        );
    }

    #[test]
    fn decoded_event_carries_keycode_flags_and_repeat() {
        let input = decode(CGEventType::KeyDown, 49, 0x000c_0000, 1);

        assert_eq!(
            input,
            Some(KeyInput {
                kind: KeyKind::Down,
                keycode: 49,
                flags: 0x000c_0000,
                autorepeat: true,
            })
        );
    }

    #[test]
    fn non_repeating_event_is_decoded_as_first_press() {
        let input = decode(CGEventType::KeyDown, 53, 0, 0);

        assert_eq!(input.map(|i| i.autorepeat), Some(false));
    }

    #[test]
    fn mouse_button_presses_are_decoded_as_pointer_input() {
        let decoded = [
            CGEventType::LeftMouseDown,
            CGEventType::RightMouseDown,
            CGEventType::OtherMouseDown,
        ]
        .map(|t| decode(t, 0, 0x0010_0008, 0).map(|i| i.kind));

        assert_eq!(decoded, [Some(KeyKind::Pointer); 3]);
    }

    #[test]
    fn other_events_are_not_decoded() {
        let decoded = [
            CGEventType::MouseMoved,
            CGEventType::LeftMouseUp,
            CGEventType::ScrollWheel,
        ]
        .map(|t| decode(t, 0, 0, 0));

        assert_eq!(decoded, [None; 3]);
    }

    #[test]
    fn tap_disabled_notifications_are_recognised() {
        let recognised = [
            CGEventType::TapDisabledByTimeout,
            CGEventType::TapDisabledByUserInput,
            CGEventType::KeyDown,
        ]
        .map(is_tap_disabled);

        assert_eq!(recognised, [true, true, false]);
    }

    fn shared() -> (Shared, crossbeam_channel::Receiver<ShortcutEvent>) {
        let (sink, events) = crossbeam_channel::unbounded();
        let shared = Shared {
            interpreter: Mutex::new(KeyInterpreter::new(&ShortcutConfig::default())),
            sink,
            stopped: AtomicBool::new(false),
        };
        (shared, events)
    }

    #[test]
    fn interpreted_key_events_reach_the_shortcut_channel() {
        let (shared, events) = shared();

        shared.handle(decode(CGEventType::FlagsChanged, 55, 0x0010_0008, 0));
        shared.handle(decode(CGEventType::MouseMoved, 0, 0, 0));
        shared.handle(decode(CGEventType::FlagsChanged, 55, 0, 0));

        assert_eq!(
            events.try_iter().collect::<Vec<_>>(),
            [ShortcutEvent::ActionTapped(Action::Dictate)]
        );
    }

    #[test]
    fn command_click_reaches_the_interpreter_and_cancels_the_tap() {
        let (shared, events) = shared();

        shared.handle(decode(CGEventType::FlagsChanged, 55, 0x0010_0008, 0));
        shared.handle(decode(CGEventType::LeftMouseDown, 0, 0x0010_0008, 0));
        shared.handle(decode(CGEventType::FlagsChanged, 55, 0, 0));

        assert_eq!(events.try_iter().count(), 0);
    }

    #[test]
    fn only_the_space_of_a_combo_shortcut_is_dropped() {
        let (shared, events) = shared();

        let dropped = [
            shared.handle(decode(CGEventType::FlagsChanged, 58, 0x0008_0020, 0)),
            shared.handle(decode(CGEventType::KeyDown, 49, 0x0008_0020, 0)),
            shared.handle(decode(CGEventType::KeyUp, 49, 0x0008_0020, 0)),
            shared.handle(decode(CGEventType::FlagsChanged, 58, 0, 0)),
            shared.handle(decode(CGEventType::KeyDown, 53, 0, 0)),
            shared.handle(decode(CGEventType::MouseMoved, 0, 0, 0)),
        ];

        assert_eq!(dropped, [false, true, true, false, false, false]);
        assert_eq!(
            events.try_iter().collect::<Vec<_>>(),
            [
                ShortcutEvent::ActionTapped(Action::EditSelection),
                ShortcutEvent::EscapePressed
            ]
        );
    }

    /// Talks to the real window server: whether the tap may be created depends on
    /// the Input Monitoring permission of the process running the tests.
    #[test]
    fn starting_either_listens_or_reports_missing_permission() {
        let (sink, events) = crossbeam_channel::unbounded();
        let config = ShortcutConfig::default();

        let started = EventTapShortcuts::start(config.clone(), sink);

        match started {
            Ok(listener) => {
                listener.update(config);
                drop(listener);
                assert!(events.try_recv().is_err());
            }
            Err(problem) => assert_eq!(problem, Problem::AccessibilityPermission),
        }
    }
}
