//! Self-healing global shortcuts: a supervisor thread keeps trying to start the
//! listener until macOS allows it, so shortcuts come alive as soon as the user
//! grants Input Monitoring / Accessibility, without restarting the app.
//!
//! The service never asks for permissions itself; prompting is the app's call.
use crate::shortcuts::EventTapShortcuts;
use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use voice_core::{Problem, ShortcutEvent};

/// How long the supervisor waits between two attempts to start the listener.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// Starts and supervises the global shortcut listener. The shortcuts are fixed, so
/// once listening there is nothing left to do but keep the listener alive.
pub struct ShortcutService;

impl ShortcutService {
    /// Spawns the supervisor using the real event tap, retrying every
    /// [`RETRY_INTERVAL`] while macOS refuses it.
    pub fn spawn(sink: Sender<ShortcutEvent>) -> ShortcutHandle {
        Self::spawn_with(sink, EventTapShortcuts::start, RETRY_INTERVAL)
    }

    /// Spawns the supervisor with an injected `start` function and retry interval.
    ///
    /// The listener is created and dropped on the supervisor thread only.
    pub(crate) fn spawn_with<L, F>(
        sink: Sender<ShortcutEvent>,
        start: F,
        retry: Duration,
    ) -> ShortcutHandle
    where
        L: 'static,
        F: FnMut(Sender<ShortcutEvent>) -> Result<L, Problem> + Send + 'static,
    {
        let (stop_tx, stop) = unbounded();
        let listening = Arc::new(AtomicBool::new(false));
        let supervisor = Supervisor {
            sink,
            start,
            retry,
            stop,
            listening: Arc::clone(&listening),
        };
        let thread = std::thread::Builder::new()
            .name("shortcut-supervisor".into())
            .spawn(move || supervisor.run())
            .map_err(|e| log::error!("cannot start the shortcut supervisor thread: {e}"))
            .ok();
        ShortcutHandle {
            stop: Some(stop_tx),
            listening,
            thread,
        }
    }
}

struct Supervisor<F> {
    sink: Sender<ShortcutEvent>,
    start: F,
    retry: Duration,
    /// Closes when the handle is dropped; nothing is ever sent on it.
    stop: Receiver<()>,
    listening: Arc<AtomicBool>,
}

impl<L, F> Supervisor<F>
where
    F: FnMut(Sender<ShortcutEvent>) -> Result<L, Problem>,
{
    fn run(mut self) {
        let mut failures: u32 = 0;
        let live = loop {
            if let Some(listener) = self.attempt(&mut failures) {
                break Some(listener);
            }
            if let Err(RecvTimeoutError::Disconnected) = self.stop.recv_timeout(self.retry) {
                break None;
            }
        };
        if live.is_some() {
            // Keep the listener until the handle goes away.
            let _ = self.stop.recv();
        }
        drop(live);
        self.listening.store(false, Ordering::Release);
    }

    /// One try at starting the listener.
    fn attempt(&mut self, failures: &mut u32) -> Option<L> {
        match (self.start)(self.sink.clone()) {
            Ok(listener) => {
                if *failures > 0 {
                    log::info!("global shortcuts available after {failures} failed attempt(s)");
                }
                *failures = 0;
                self.listening.store(true, Ordering::Release);
                Some(listener)
            }
            Err(problem) => {
                if *failures == 0 {
                    log::warn!("global shortcuts unavailable, retrying: {problem}");
                } else {
                    log::debug!("global shortcuts still unavailable: {problem}");
                }
                *failures = failures.saturating_add(1);
                None
            }
        }
    }
}

/// Controls a running [`ShortcutService`]. Dropping it stops the supervisor and
/// the listener.
pub struct ShortcutHandle {
    /// `None` only while dropping, so the supervisor sees the channel close.
    stop: Option<Sender<()>>,
    listening: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ShortcutHandle {
    /// Whether the global shortcuts are being listened to right now.
    pub fn is_listening(&self) -> bool {
        self.listening.load(Ordering::Acquire)
    }
}

impl Drop for ShortcutHandle {
    fn drop(&mut self) {
        self.stop = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Instant;
    use voice_core::Action;

    const WAIT: Duration = Duration::from_secs(5);
    const FAST: Duration = Duration::from_millis(10);
    const NEVER: Duration = Duration::from_secs(3600);

    /// Records what the supervisor does with the listeners it starts.
    #[derive(Default)]
    struct Log {
        attempts: Mutex<usize>,
        dropped: AtomicBool,
    }

    impl Log {
        fn attempts(&self) -> usize {
            *self.attempts.lock().unwrap()
        }
    }

    struct FakeListener(Arc<Log>);

    impl Drop for FakeListener {
        fn drop(&mut self) {
            self.0.dropped.store(true, Ordering::SeqCst);
        }
    }

    /// A `start` that refuses `refusals` times, then succeeds, sending
    /// a dictation tap through the sink it was given.
    fn start_after(
        refusals: usize,
        log: &Arc<Log>,
    ) -> impl FnMut(Sender<ShortcutEvent>) -> Result<FakeListener, Problem> + Send {
        let mut refusals_left = refusals;
        let log = Arc::clone(log);
        move |sink| {
            *log.attempts.lock().unwrap() += 1;
            if refusals_left == 0 {
                let _ = sink.send(ShortcutEvent::ActionTapped(Action::Dictate));
                Ok(FakeListener(Arc::clone(&log)))
            } else {
                refusals_left -= 1;
                Err(Problem::AccessibilityPermission)
            }
        }
    }

    fn spawn(refusals: usize, retry: Duration) -> (ShortcutHandle, Arc<Log>) {
        let log = Arc::new(Log::default());
        let (sink, _) = unbounded();
        let handle = ShortcutService::spawn_with(sink, start_after(refusals, &log), retry);
        (handle, log)
    }

    fn wait_until(what: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + WAIT;
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn listens_right_away_when_the_listener_starts() {
        let (handle, log) = spawn(0, NEVER);

        wait_until("listening", || handle.is_listening());
        assert_eq!(log.attempts(), 1);
    }

    #[test]
    fn retries_until_the_listener_starts() {
        let (handle, log) = spawn(3, FAST);

        wait_until("listening", || handle.is_listening());
        assert_eq!(log.attempts(), 4);
    }

    #[test]
    fn waits_the_retry_interval_between_attempts() {
        let (handle, log) = spawn(usize::MAX, NEVER);

        wait_until("first attempt", || log.attempts() == 1);
        std::thread::sleep(Duration::from_millis(100));

        assert_eq!(log.attempts(), 1);
        assert!(!handle.is_listening());
    }

    #[test]
    fn retries_again_after_the_interval_elapses() {
        let (handle, log) = spawn(1, Duration::from_millis(200));
        wait_until("first attempt", || log.attempts() == 1);
        let refused_at = Instant::now();

        wait_until("listening", || handle.is_listening());

        assert!(refused_at.elapsed() >= Duration::from_millis(150));
        assert_eq!(log.attempts(), 2);
    }

    #[test]
    fn events_from_the_listener_reach_the_sink() {
        let log = Arc::new(Log::default());
        let (sink, events) = unbounded();

        let _handle = ShortcutService::spawn_with(sink, start_after(0, &log), NEVER);

        assert_eq!(
            events.recv_timeout(WAIT),
            Ok(ShortcutEvent::ActionTapped(Action::Dictate))
        );
    }

    #[test]
    fn a_live_listener_is_never_restarted() {
        let (handle, log) = spawn(0, FAST);
        wait_until("listening", || handle.is_listening());

        std::thread::sleep(Duration::from_millis(50));

        assert_eq!(log.attempts(), 1);
        assert!(!log.dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn dropping_the_handle_stops_the_listener() {
        let (handle, log) = spawn(0, NEVER);
        wait_until("listening", || handle.is_listening());

        drop(handle);

        assert!(log.dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn dropping_the_handle_stops_retrying() {
        let (handle, log) = spawn(usize::MAX, FAST);
        wait_until("a few attempts", || log.attempts() >= 3);

        drop(handle);
        let attempts = log.attempts();
        std::thread::sleep(Duration::from_millis(50));

        assert_eq!(log.attempts(), attempts);
    }

    #[test]
    fn not_listening_after_the_supervisor_stops() {
        let (handle, _log) = spawn(0, NEVER);
        wait_until("listening", || handle.is_listening());
        let listening = Arc::clone(&handle.listening);

        drop(handle);

        assert!(!listening.load(Ordering::SeqCst));
    }

    /// Talks to the real window server: whether the tap may be created depends on
    /// the Input Monitoring permission of the process running the tests.
    #[test]
    fn real_service_starts_and_stops_cleanly() {
        let (sink, events) = unbounded();

        let handle = ShortcutService::spawn(sink);
        drop(handle);

        assert!(events.try_recv().is_err());
    }
}
