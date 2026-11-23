//! Global shortcut listener: a `CGEventTap` on its own thread.
//!
//! The tap thread only decodes events, forwards them to [`KeyInterpreter`] and
//! drops the ones it says to consume (the Space of a combo shortcut); every
//! mapping rule lives there.
//!
//! The tap is created through the C API directly: it also listens to trackpad
//! gesture events, whose type numbers the `core-graphics` event-type enum cannot hold.
use crate::keys::{KeyDecision, KeyInput, KeyInterpreter, KeyKind, ShortcutConfig, Surroundings};
use crate::surroundings::SystemSurroundings;
use core_foundation::base::TCFType;
use core_foundation::mach_port::{CFMachPort, CFMachPortRef};
use core_foundation::runloop::{kCFRunLoopCommonModes, kCFRunLoopDefaultMode, CFRunLoop};
use core_graphics::event::{CGEventField, EventField};
use crossbeam_channel::Sender;
use std::cell::Cell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;
use voice_core::{Problem, ShortcutEvent};

/// Raw `CGEventType` values (gesture types are `NSEventType` numbers the window
/// server also delivers to session taps).
mod event_type {
    pub const LEFT_MOUSE_DOWN: u32 = 1;
    pub const RIGHT_MOUSE_DOWN: u32 = 3;
    pub const KEY_DOWN: u32 = 10;
    pub const KEY_UP: u32 = 11;
    pub const FLAGS_CHANGED: u32 = 12;
    pub const ROTATE: u32 = 18;
    pub const BEGIN_GESTURE: u32 = 19;
    pub const END_GESTURE: u32 = 20;
    pub const SCROLL_WHEEL: u32 = 22;
    pub const OTHER_MOUSE_DOWN: u32 = 25;
    pub const GESTURE: u32 = 29;
    pub const MAGNIFY: u32 = 30;
    pub const SWIPE: u32 = 31;
    pub const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
    pub const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

    /// Every event type the tap listens to.
    pub const WATCHED: [u32; 13] = [
        KEY_DOWN,
        KEY_UP,
        FLAGS_CHANGED,
        LEFT_MOUSE_DOWN,
        RIGHT_MOUSE_DOWN,
        OTHER_MOUSE_DOWN,
        SCROLL_WHEEL,
        ROTATE,
        BEGIN_GESTURE,
        END_GESTURE,
        GESTURE,
        MAGNIFY,
        SWIPE,
    ];
}

/// The `CGEventMask` of [`event_type::WATCHED`].
fn event_mask() -> u64 {
    event_type::WATCHED
        .iter()
        .fold(0, |mask, kind| mask | 1 << kind)
}

type CGEventRef = *mut c_void;
type TapCallback = unsafe extern "C" fn(
    proxy: *const c_void,
    event_type: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef;

/// `kCGSessionEventTap`, `kCGHeadInsertEventTap`, `kCGEventTapOptionDefault`.
const SESSION_TAP: u32 = 1;
const HEAD_INSERT: u32 = 0;
const TAP_CAN_DROP: u32 = 0;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: TapCallback,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: CGEventField) -> i64;
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventGetTimestamp(event: CGEventRef) -> u64;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFMachPortInvalidate(port: CFMachPortRef);
}

/// Stops the tap for good, so its callback can no longer run.
fn invalidate(port: &CFMachPort) {
    // SAFETY: `port` is a live mach port we own.
    unsafe { CFMachPortInvalidate(port.as_concrete_TypeRef()) };
}

#[repr(C)]
#[derive(Default)]
struct MachTimebase {
    numer: u32,
    denom: u32,
}

/// `CLOCK_UPTIME_RAW`: `mach_absolute_time` in nanoseconds.
const CLOCK_UPTIME_RAW: u32 = 8;

extern "C" {
    fn mach_absolute_time() -> u64;
    fn mach_timebase_info(info: *mut MachTimebase) -> i32;
    fn clock_gettime_nsec_np(clock: u32) -> u64;
}

/// Decodes the parts of a `CGEvent` the interpreter needs. `None` for event
/// types that are neither keyboard events nor pointer activity.
pub(crate) fn decode(
    event_type: u32,
    keycode: i64,
    flags: u64,
    autorepeat: i64,
    timestamp: Duration,
) -> Option<KeyInput> {
    use event_type::*;
    let kind = match event_type {
        KEY_DOWN => KeyKind::Down,
        KEY_UP => KeyKind::Up,
        FLAGS_CHANGED => KeyKind::FlagsChanged,
        LEFT_MOUSE_DOWN | RIGHT_MOUSE_DOWN | OTHER_MOUSE_DOWN | SCROLL_WHEEL | ROTATE
        | BEGIN_GESTURE | END_GESTURE | GESTURE | MAGNIFY | SWIPE => KeyKind::Pointer,
        _ => return None,
    };
    Some(KeyInput {
        kind,
        keycode: u16::try_from(keycode).unwrap_or(u16::MAX),
        flags,
        autorepeat: autorepeat != 0,
        timestamp,
    })
}

/// macOS turns a tap off when its callback is slow or on secure input.
fn is_tap_disabled(event_type: u32) -> bool {
    matches!(
        event_type,
        event_type::TAP_DISABLED_BY_TIMEOUT | event_type::TAP_DISABLED_BY_USER_INPUT
    )
}

/// Turns a raw event timestamp into nanoseconds since boot.
///
/// Apple documents the timestamp as nanoseconds, yet real input events carry
/// `mach_absolute_time` ticks on Apple silicon (where a tick is not a nanosecond).
/// An event is always recent, so whichever clock it is closest to right now is
/// the clock it was stamped with. Events without a timestamp get "now".
fn event_nanos(raw: u64, ticks_now: u64, nanos_now: u64, timebase: (u32, u32)) -> u64 {
    if raw == 0 {
        return nanos_now;
    }
    if raw.abs_diff(ticks_now) < raw.abs_diff(nanos_now) {
        let (numer, denom) = timebase;
        let nanos = u128::from(raw) * u128::from(numer) / u128::from(denom.max(1));
        u64::try_from(nanos).unwrap_or(u64::MAX)
    } else {
        raw
    }
}

fn timebase() -> (u32, u32) {
    static TIMEBASE: OnceLock<(u32, u32)> = OnceLock::new();
    *TIMEBASE.get_or_init(|| {
        let mut info = MachTimebase::default();
        // SAFETY: `info` is a valid out pointer for the duration of the call.
        let ok = unsafe { mach_timebase_info(&mut info) } == 0;
        if ok && info.denom != 0 {
            (info.numer, info.denom)
        } else {
            (1, 1)
        }
    })
}

fn event_time(raw: u64) -> Duration {
    // SAFETY: plain clock queries without arguments that can be invalid.
    let (ticks_now, nanos_now) = unsafe {
        (
            mach_absolute_time(),
            clock_gettime_nsec_np(CLOCK_UPTIME_RAW),
        )
    };
    Duration::from_nanos(event_nanos(raw, ticks_now, nanos_now, timebase()))
}

struct Shared {
    interpreter: Mutex<KeyInterpreter>,
    around: Box<dyn Surroundings + Send + Sync>,
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
        let KeyDecision { event, consume } = self.interpreter().on_key(input, &*self.around);
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
            around: Box::new(SystemSurroundings),
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

/// What the tap callback works with. Lives on the tap thread for as long as the tap.
struct TapState {
    shared: Arc<Shared>,
    /// The tap's own port, so the callback can switch it back on.
    port: Cell<CFMachPortRef>,
}

/// Called by the window server for every watched event.
unsafe extern "C" fn tap_callback(
    _proxy: *const c_void,
    event_type: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    // SAFETY: `user_info` is the `TapState` that `run_tap` keeps alive until
    // the tap is gone, and only this thread touches it.
    let state = unsafe { &*(user_info as *const TapState) };
    if is_tap_disabled(event_type) {
        let port = state.port.get();
        if !port.is_null() {
            // SAFETY: the port belongs to the tap, which outlives its callback.
            unsafe { CGEventTapEnable(port, true) };
        }
        return event;
    }
    if event.is_null() {
        return event;
    }
    // SAFETY: `event` is the live event the callback was handed.
    let input = unsafe {
        decode(
            event_type,
            CGEventGetIntegerValueField(event, EventField::KEYBOARD_EVENT_KEYCODE),
            CGEventGetFlags(event),
            CGEventGetIntegerValueField(event, EventField::KEYBOARD_EVENT_AUTOREPEAT),
            event_time(CGEventGetTimestamp(event)),
        )
    };
    if state.shared.handle(input) {
        std::ptr::null_mut()
    } else {
        event
    }
}

/// Body of the tap thread: reports its run loop (or `None` on failure), then
/// runs until the listener stops it.
fn run_tap(shared: &Arc<Shared>, ready: &Sender<Option<CFRunLoop>>) {
    let state = Box::new(TapState {
        shared: Arc::clone(shared),
        port: Cell::new(std::ptr::null_mut()),
    });
    let user_info = &*state as *const TapState as *mut c_void;
    // SAFETY: the callback matches the C signature, and `user_info` points to
    // `state`, which outlives the tap (it is dropped at the end of this function,
    // after the port is invalidated).
    let raw = unsafe {
        CGEventTapCreate(
            SESSION_TAP,
            HEAD_INSERT,
            TAP_CAN_DROP,
            event_mask(),
            tap_callback,
            user_info,
        )
    };
    if raw.is_null() {
        let _ = ready.send(None);
        return;
    }
    // SAFETY: a non-null port returned under the Create rule; we own it now.
    let port = unsafe { CFMachPort::wrap_under_create_rule(raw) };
    let Ok(source) = port.create_runloop_source(0) else {
        invalidate(&port);
        let _ = ready.send(None);
        return;
    };
    state.port.set(raw);
    let run_loop = CFRunLoop::get_current();
    // SAFETY: reading an immutable CoreFoundation constant.
    run_loop.add_source(&source, unsafe { kCFRunLoopCommonModes });
    // SAFETY: `raw` is the live port of our tap.
    unsafe { CGEventTapEnable(raw, true) };
    let _ = ready.send(Some(run_loop));
    // A stop requested before the loop starts running would be lost, so the
    // flag is checked between bounded runs as well.
    while !shared.stopped.load(Ordering::Acquire) {
        // SAFETY: reading an immutable CoreFoundation constant.
        CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, STOP_POLL, false);
    }
    invalidate(&port);
    state.port.set(std::ptr::null_mut());
    drop(source);
    drop(state);
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

    use event_type::*;

    const T: Duration = Duration::from_millis(7);

    #[test]
    fn keyboard_event_types_are_decoded() {
        let decoded =
            [KEY_DOWN, KEY_UP, FLAGS_CHANGED].map(|t| decode(t, 61, 0x40, 1, T).map(|i| i.kind));

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
        let input = decode(KEY_DOWN, 49, 0x000c_0000, 1, T);

        assert_eq!(
            input,
            Some(KeyInput {
                kind: KeyKind::Down,
                keycode: 49,
                flags: 0x000c_0000,
                autorepeat: true,
                timestamp: T,
            })
        );
    }

    #[test]
    fn non_repeating_event_is_decoded_as_first_press() {
        let input = decode(KEY_DOWN, 53, 0, 0, T);

        assert_eq!(input.map(|i| i.autorepeat), Some(false));
    }

    #[test]
    fn mouse_presses_scrolling_and_gestures_are_decoded_as_pointer_input() {
        let pointer = [
            LEFT_MOUSE_DOWN,
            RIGHT_MOUSE_DOWN,
            OTHER_MOUSE_DOWN,
            SCROLL_WHEEL,
            ROTATE,
            BEGIN_GESTURE,
            END_GESTURE,
            GESTURE,
            MAGNIFY,
            SWIPE,
        ];

        let decoded = pointer.map(|t| decode(t, 0, 0x0010_0008, 0, T).map(|i| i.kind));

        assert_eq!(decoded, [Some(KeyKind::Pointer); 10]);
    }

    #[test]
    fn other_events_are_not_decoded() {
        let mouse_moved = 5;
        let left_mouse_up = 2;
        let decoded =
            [mouse_moved, left_mouse_up, TAP_DISABLED_BY_TIMEOUT].map(|t| decode(t, 0, 0, 0, T));

        assert_eq!(decoded, [None; 3]);
    }

    #[test]
    fn the_tap_listens_to_exactly_the_decoded_event_types() {
        let mask = event_mask();

        let listened: Vec<u32> = (0..64).filter(|t| mask & 1 << t != 0).collect();
        let decoded: Vec<u32> = (0..64)
            .filter(|t| decode(*t, 0, 0, 0, T).is_some())
            .collect();

        assert_eq!(listened, decoded);
        assert_eq!(listened.len(), WATCHED.len());
    }

    #[test]
    fn tap_disabled_notifications_are_recognised() {
        let recognised = [
            TAP_DISABLED_BY_TIMEOUT,
            TAP_DISABLED_BY_USER_INPUT,
            KEY_DOWN,
        ]
        .map(is_tap_disabled);

        assert_eq!(recognised, [true, true, false]);
    }

    #[test]
    fn mach_ticks_are_converted_to_nanoseconds() {
        // Apple silicon: 24 MHz ticks, 125/3 ns each.
        let nanos = event_nanos(24_000_000, 24_000_100, 1_000_004_000, (125, 3));

        assert_eq!(nanos, 1_000_000_000);
    }

    #[test]
    fn nanosecond_timestamps_are_kept() {
        let nanos = event_nanos(1_000_000_000, 24_000_100, 1_000_004_000, (125, 3));

        assert_eq!(nanos, 1_000_000_000);
    }

    #[test]
    fn identical_clocks_keep_the_value() {
        let nanos = event_nanos(5_000, 5_100, 5_100, (1, 1));

        assert_eq!(nanos, 5_000);
    }

    #[test]
    fn an_event_without_timestamp_happened_now() {
        let nanos = event_nanos(0, 24_000_100, 1_000_004_000, (125, 3));

        assert_eq!(nanos, 1_000_004_000);
    }

    #[test]
    fn real_event_time_follows_the_uptime_clock() {
        // SAFETY: plain clock query.
        let before = Duration::from_nanos(unsafe { clock_gettime_nsec_np(CLOCK_UPTIME_RAW) });
        // SAFETY: plain clock query.
        let raw = unsafe { mach_absolute_time() };

        let at = event_time(raw);

        assert!(
            at >= before && at - before < Duration::from_secs(1),
            "{at:?} vs {before:?}"
        );
    }

    /// No secure input; the frontmost app is whatever the cell says right now.
    struct World {
        app: std::sync::atomic::AtomicI32,
    }

    impl Surroundings for Arc<World> {
        fn secure_input(&self) -> bool {
            false
        }
        fn frontmost_app(&self) -> Option<i32> {
            Some(self.app.load(Ordering::SeqCst))
        }
    }

    fn shared_in(world: Arc<World>) -> (Shared, crossbeam_channel::Receiver<ShortcutEvent>) {
        let (sink, events) = crossbeam_channel::unbounded();
        let shared = Shared {
            interpreter: Mutex::new(KeyInterpreter::new(&ShortcutConfig::default())),
            around: Box::new(world),
            sink,
            stopped: AtomicBool::new(false),
        };
        (shared, events)
    }

    fn shared() -> (Shared, crossbeam_channel::Receiver<ShortcutEvent>) {
        shared_in(Arc::new(World { app: 1.into() }))
    }

    fn at(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn interpreted_key_events_reach_the_shortcut_channel() {
        let (shared, events) = shared();

        shared.handle(decode(FLAGS_CHANGED, 55, 0x0010_0008, 0, at(0)));
        shared.handle(decode(5, 0, 0, 0, at(100)));
        shared.handle(decode(FLAGS_CHANGED, 55, 0, 0, at(200)));

        assert_eq!(
            events.try_iter().collect::<Vec<_>>(),
            [ShortcutEvent::ActionTapped(Action::Dictate)]
        );
    }

    #[test]
    fn command_click_reaches_the_interpreter_and_cancels_the_tap() {
        let (shared, events) = shared();

        shared.handle(decode(FLAGS_CHANGED, 55, 0x0010_0008, 0, at(0)));
        shared.handle(decode(LEFT_MOUSE_DOWN, 0, 0x0010_0008, 0, at(100)));
        shared.handle(decode(FLAGS_CHANGED, 55, 0, 0, at(200)));

        assert_eq!(events.try_iter().count(), 0);
    }

    #[test]
    fn command_scroll_cancels_the_tap() {
        let (shared, events) = shared();

        shared.handle(decode(FLAGS_CHANGED, 55, 0x0010_0008, 0, at(0)));
        shared.handle(decode(SCROLL_WHEEL, 0, 0x0010_0008, 0, at(100)));
        shared.handle(decode(FLAGS_CHANGED, 55, 0, 0, at(200)));

        assert_eq!(events.try_iter().count(), 0);
    }

    #[test]
    fn command_pinch_cancels_the_tap() {
        let (shared, events) = shared();

        shared.handle(decode(FLAGS_CHANGED, 55, 0x0010_0008, 0, at(0)));
        shared.handle(decode(MAGNIFY, 0, 0x0010_0008, 0, at(100)));
        shared.handle(decode(FLAGS_CHANGED, 55, 0, 0, at(200)));

        assert_eq!(events.try_iter().count(), 0);
    }

    #[test]
    fn command_held_over_a_second_is_not_a_tap() {
        let (shared, events) = shared();

        shared.handle(decode(FLAGS_CHANGED, 55, 0x0010_0008, 0, at(0)));
        shared.handle(decode(FLAGS_CHANGED, 55, 0, 0, at(1_500)));

        assert_eq!(events.try_iter().count(), 0);
    }

    #[test]
    fn command_tab_to_another_app_is_not_a_tap() {
        let world = Arc::new(World { app: 1.into() });
        let (shared, events) = shared_in(Arc::clone(&world));

        shared.handle(decode(FLAGS_CHANGED, 55, 0x0010_0008, 0, at(0)));
        // Tab itself never reaches the session tap; only the app switch is visible.
        world.app.store(2, Ordering::SeqCst);
        shared.handle(decode(FLAGS_CHANGED, 55, 0, 0, at(300)));

        assert_eq!(events.try_iter().count(), 0);
    }

    #[test]
    fn only_the_space_of_a_combo_shortcut_is_dropped() {
        let (shared, events) = shared();

        let dropped = [
            shared.handle(decode(FLAGS_CHANGED, 58, 0x0008_0020, 0, at(0))),
            shared.handle(decode(KEY_DOWN, 49, 0x0008_0020, 0, at(10))),
            shared.handle(decode(KEY_UP, 49, 0x0008_0020, 0, at(20))),
            shared.handle(decode(FLAGS_CHANGED, 58, 0, 0, at(30))),
            shared.handle(decode(KEY_DOWN, 53, 0, 0, T)),
            shared.handle(decode(5, 0, 0, 0, at(100))),
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
