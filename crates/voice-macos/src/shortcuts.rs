//! Global shortcut listener: a listen-only `CGEventTap` on its own thread.
//!
//! The tap thread only decodes events and forwards them to [`KeyInterpreter`];
//! every mapping rule lives there.
use crate::keys::{KeyInput, KeyInterpreter, KeyKind, ShortcutConfig};
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
/// types that are not keyboard events.
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

    /// Interprets one key event and forwards what it means.
    fn handle(&self, input: Option<KeyInput>) {
        if let Some(input) = input {
            let produced = self.interpreter().on_key(input);
            self.emit(produced);
        }
    }

    fn emit(&self, event: Option<ShortcutEvent>) {
        if let Some(event) = event {
            // The receiver is gone only while the app shuts down.
            let _ = self.sink.send(event);
        }
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
        callback_shared.handle(decode(
            event_type,
            event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE),
            event.get_flags().bits(),
            event.get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT),
        ));
        CallbackResult::Keep
    };
    let tap = CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::ListenOnly,
        vec![
            CGEventType::KeyDown,
            CGEventType::KeyUp,
            CGEventType::FlagsChanged,
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
        let produced = self.shared.interpreter().reconfigure(&config);
        self.shared.emit(produced);
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
    use voice_core::Shortcut;

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
    fn non_keyboard_events_are_not_decoded() {
        let decoded = decode(CGEventType::MouseMoved, 0, 0, 0);

        assert_eq!(decoded, None);
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

    #[test]
    fn interpreted_key_events_reach_the_shortcut_channel() {
        let (sink, events) = crossbeam_channel::unbounded();
        let shared = Shared {
            interpreter: Mutex::new(KeyInterpreter::new(&ShortcutConfig {
                push_to_talk: Shortcut::RightOption,
                hands_free: Shortcut::ControlOptionSpace,
            })),
            sink,
            stopped: AtomicBool::new(false),
        };

        shared.handle(decode(CGEventType::FlagsChanged, 61, 0x0008_0040, 0));
        shared.handle(decode(CGEventType::MouseMoved, 0, 0, 0));

        assert_eq!(
            events.try_iter().collect::<Vec<_>>(),
            [ShortcutEvent::PushToTalkDown]
        );
    }

    /// Talks to the real window server: whether the tap may be created depends on
    /// the Input Monitoring permission of the process running the tests.
    #[test]
    fn starting_either_listens_or_reports_missing_permission() {
        let (sink, events) = crossbeam_channel::unbounded();
        let config = ShortcutConfig {
            push_to_talk: Shortcut::RightOption,
            hands_free: Shortcut::ControlOptionSpace,
        };

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
