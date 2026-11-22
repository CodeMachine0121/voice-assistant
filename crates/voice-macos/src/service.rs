//! Self-healing global shortcuts: a supervisor thread keeps trying to start the
//! listener until macOS allows it, so shortcuts come alive as soon as the user
//! grants Input Monitoring / Accessibility, without restarting the app.
//!
//! The service never asks for permissions itself; prompting is the app's call.
use crate::keys::ShortcutConfig;
use crate::shortcuts::{EventTapShortcuts, ShortcutListener};
use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use voice_core::{Problem, ShortcutEvent};

/// How long the supervisor waits between two attempts to start the listener.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// What the supervisor needs from a running listener.
pub(crate) trait LiveShortcuts {
    fn update(&self, config: ShortcutConfig);
}

impl LiveShortcuts for ShortcutListener {
    fn update(&self, config: ShortcutConfig) {
        ShortcutListener::update(self, config);
    }
}

/// Starts and supervises the global shortcut listener.
pub struct ShortcutService;

impl ShortcutService {
    /// Spawns the supervisor using the real event tap, retrying every
    /// [`RETRY_INTERVAL`] while macOS refuses it.
    pub fn spawn(config: ShortcutConfig, sink: Sender<ShortcutEvent>) -> ShortcutHandle {
        Self::spawn_with(config, sink, EventTapShortcuts::start, RETRY_INTERVAL)
    }

    /// Spawns the supervisor with an injected `start` function and retry interval.
    ///
    /// The listener is created, updated and dropped on the supervisor thread only.
    pub(crate) fn spawn_with<L, F>(
        config: ShortcutConfig,
        sink: Sender<ShortcutEvent>,
        start: F,
        retry: Duration,
    ) -> ShortcutHandle
    where
        L: LiveShortcuts + 'static,
        F: FnMut(ShortcutConfig, Sender<ShortcutEvent>) -> Result<L, Problem> + Send + 'static,
    {
        let (configs_tx, configs) = unbounded();
        let listening = Arc::new(AtomicBool::new(false));
        let supervisor = Supervisor {
            pending: config,
            sink,
            start,
            retry,
            configs,
            listening: Arc::clone(&listening),
        };
        let thread = std::thread::Builder::new()
            .name("shortcut-supervisor".into())
            .spawn(move || supervisor.run())
            .map_err(|e| log::error!("cannot start the shortcut supervisor thread: {e}"))
            .ok();
        ShortcutHandle {
            configs: Some(configs_tx),
            listening,
            thread,
        }
    }
}

struct Supervisor<F> {
    /// The configuration the listener runs with, or will start with.
    pending: ShortcutConfig,
    sink: Sender<ShortcutEvent>,
    start: F,
    retry: Duration,
    configs: Receiver<ShortcutConfig>,
    listening: Arc<AtomicBool>,
}

impl<L, F> Supervisor<F>
where
    L: LiveShortcuts,
    F: FnMut(ShortcutConfig, Sender<ShortcutEvent>) -> Result<L, Problem>,
{
    fn run(mut self) {
        let mut live: Option<L> = None;
        let mut failures: u32 = 0;
        loop {
            if live.is_none() {
                live = self.attempt(&mut failures);
            }
            let next = match live {
                Some(_) => self
                    .configs
                    .recv()
                    .map_err(|_| RecvTimeoutError::Disconnected),
                None => self.configs.recv_timeout(self.retry),
            };
            match next {
                Ok(config) => {
                    if let Some(listener) = &live {
                        listener.update(config.clone());
                    }
                    self.pending = config;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        drop(live);
        self.listening.store(false, Ordering::Release);
    }

    /// One try at starting the listener with the pending configuration.
    fn attempt(&mut self, failures: &mut u32) -> Option<L> {
        match (self.start)(self.pending.clone(), self.sink.clone()) {
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
    configs: Option<Sender<ShortcutConfig>>,
    listening: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ShortcutHandle {
    /// Switches the live listener to new shortcuts, or keeps them for the next
    /// start attempt while it is not listening.
    pub fn apply(&self, config: ShortcutConfig) {
        if let Some(configs) = &self.configs {
            // The supervisor is gone only if its thread could not be created.
            let _ = configs.send(config);
        }
    }

    /// Whether the global shortcuts are being listened to right now.
    pub fn is_listening(&self) -> bool {
        self.listening.load(Ordering::Acquire)
    }
}

impl Drop for ShortcutHandle {
    fn drop(&mut self) {
        self.configs = None;
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
    use voice_core::Shortcut;

    const WAIT: Duration = Duration::from_secs(5);
    const FAST: Duration = Duration::from_millis(10);
    const NEVER: Duration = Duration::from_secs(3600);

    fn config(push_to_talk: Shortcut) -> ShortcutConfig {
        ShortcutConfig {
            push_to_talk,
            hands_free: Shortcut::ControlOptionSpace,
        }
    }

    /// Records what the supervisor does with the listeners it starts.
    #[derive(Default)]
    struct Log {
        attempts: Mutex<Vec<ShortcutConfig>>,
        updates: Mutex<Vec<ShortcutConfig>>,
        dropped: AtomicBool,
    }

    impl Log {
        fn attempts(&self) -> Vec<ShortcutConfig> {
            self.attempts.lock().unwrap().clone()
        }
        fn updates(&self) -> Vec<ShortcutConfig> {
            self.updates.lock().unwrap().clone()
        }
    }

    struct FakeListener(Arc<Log>);

    impl LiveShortcuts for FakeListener {
        fn update(&self, config: ShortcutConfig) {
            self.0.updates.lock().unwrap().push(config);
        }
    }

    impl Drop for FakeListener {
        fn drop(&mut self) {
            self.0.dropped.store(true, Ordering::SeqCst);
        }
    }

    /// A `start` that refuses `refusals` times, then succeeds, sending
    /// `PushToTalkDown` through the sink it was given.
    fn start_after(
        refusals: usize,
        log: &Arc<Log>,
    ) -> impl FnMut(ShortcutConfig, Sender<ShortcutEvent>) -> Result<FakeListener, Problem> + Send
    {
        let mut refusals_left = refusals;
        let log = Arc::clone(log);
        move |config, sink| {
            log.attempts.lock().unwrap().push(config);
            if refusals_left == 0 {
                let _ = sink.send(ShortcutEvent::PushToTalkDown);
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
        let handle = ShortcutService::spawn_with(
            config(Shortcut::RightOption),
            sink,
            start_after(refusals, &log),
            retry,
        );
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
        assert_eq!(log.attempts(), [config(Shortcut::RightOption)]);
    }

    #[test]
    fn retries_until_the_listener_starts() {
        let (handle, log) = spawn(3, FAST);

        wait_until("listening", || handle.is_listening());
        assert_eq!(log.attempts().len(), 4);
    }

    #[test]
    fn waits_the_retry_interval_between_attempts() {
        let (handle, log) = spawn(usize::MAX, NEVER);

        wait_until("first attempt", || log.attempts().len() == 1);
        std::thread::sleep(Duration::from_millis(100));

        assert_eq!(log.attempts().len(), 1);
        assert!(!handle.is_listening());
    }

    #[test]
    fn retries_again_after_the_interval_elapses() {
        let (handle, log) = spawn(1, Duration::from_millis(200));
        wait_until("first attempt", || log.attempts().len() == 1);
        let refused_at = Instant::now();

        wait_until("listening", || handle.is_listening());

        assert!(refused_at.elapsed() >= Duration::from_millis(150));
        assert_eq!(log.attempts().len(), 2);
    }

    #[test]
    fn events_from_the_listener_reach_the_sink() {
        let log = Arc::new(Log::default());
        let (sink, events) = unbounded();

        let _handle = ShortcutService::spawn_with(
            config(Shortcut::RightOption),
            sink,
            start_after(0, &log),
            NEVER,
        );

        assert_eq!(events.recv_timeout(WAIT), Ok(ShortcutEvent::PushToTalkDown));
    }

    #[test]
    fn new_config_goes_to_the_live_listener_without_restarting_it() {
        let (handle, log) = spawn(0, NEVER);
        wait_until("listening", || handle.is_listening());

        handle.apply(config(Shortcut::Fn));

        wait_until("update", || !log.updates().is_empty());
        assert_eq!(log.updates(), [config(Shortcut::Fn)]);
        assert_eq!(log.attempts().len(), 1);
    }

    #[test]
    fn new_config_while_not_listening_is_used_for_the_next_attempt() {
        let (handle, log) = spawn(1, NEVER);
        wait_until("first attempt", || log.attempts().len() == 1);

        handle.apply(config(Shortcut::RightCommand));

        wait_until("listening", || handle.is_listening());
        assert_eq!(
            log.attempts(),
            [
                config(Shortcut::RightOption),
                config(Shortcut::RightCommand)
            ]
        );
        assert!(log.updates().is_empty());
    }

    #[test]
    fn newest_config_survives_further_refusals() {
        let (handle, log) = spawn(2, FAST);
        wait_until("first attempt", || !log.attempts().is_empty());
        handle.apply(config(Shortcut::Fn));

        wait_until("listening", || handle.is_listening());

        assert_eq!(log.attempts().last(), Some(&config(Shortcut::Fn)));
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
        wait_until("a few attempts", || log.attempts().len() >= 3);

        drop(handle);
        let attempts = log.attempts().len();
        std::thread::sleep(Duration::from_millis(50));

        assert_eq!(log.attempts().len(), attempts);
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

        let handle = ShortcutService::spawn(config(Shortcut::RightOption), sink);
        handle.apply(config(Shortcut::Fn));
        drop(handle);

        assert!(events.try_recv().is_err());
    }
}
