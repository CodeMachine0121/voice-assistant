//! State of the running app, apart from how it is drawn.
use crate::actions::save_settings;
use crate::indicator::IndicatorState;
use crate::permissions_view::PermissionSnapshot;
use crate::settings_form::SettingsForm;
use crate::tasks::{
    download_target, spawn_download, spawn_list_models, ModelsState, TaskEvent, TaskState,
};
use crate::tray::TrayCommand;
use crate::wiring::Services;
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::time::Instant;
use voice_core::{History, Settings, Status};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Saved,
    Problem(String),
}

/// One opening of the Settings window: the form being edited and its background work.
pub struct SettingsSession {
    pub form: SettingsForm,
    stored: Settings,
    /// Input devices, read when the window opened.
    pub devices: Vec<String>,
    pub tasks: TaskState,
    pub notice: Option<Notice>,
    /// What macOS allows, re-read while the window is open.
    pub permissions: PermissionSnapshot,
    events_tx: Sender<TaskEvent>,
    events_rx: Receiver<TaskEvent>,
}

impl SettingsSession {
    pub fn open(services: &Services) -> Self {
        let stored = services.settings_store.load();
        let (events_tx, events_rx) = unbounded();
        Self {
            form: SettingsForm::from_settings(&stored),
            stored,
            devices: (services.list_input_devices)(),
            tasks: TaskState::default(),
            notice: None,
            permissions: PermissionSnapshot::take(&services.permissions, Instant::now()),
            events_tx,
            events_rx,
        }
    }

    /// Device names for the picker; a chosen device that is not plugged in stays listed.
    pub fn microphone_options(&self) -> Vec<String> {
        let mut options = self.devices.clone();
        if let Some(chosen) = &self.form.microphone {
            if !options.contains(chosen) {
                options.push(chosen.clone());
            }
        }
        options
    }

    pub fn refresh_models(&mut self, services: &Services) {
        if self.tasks.models == ModelsState::Loading {
            return;
        }
        self.tasks.models = ModelsState::Loading;
        spawn_list_models(
            services.list_models.clone(),
            self.form.assistant_url.trim().to_string(),
            self.events_tx.clone(),
            services.repaint.clone(),
        );
    }

    pub fn download_model(&mut self, services: &Services) {
        if self.tasks.download.is_running() {
            return;
        }
        self.tasks.apply(TaskEvent::DownloadProgress {
            done: 0,
            total: None,
        });
        let target = download_target(
            &self.form.speech_model_path,
            &(services.default_model_path)(),
        );
        spawn_download(
            services.download_model.clone(),
            target,
            self.events_tx.clone(),
            services.repaint.clone(),
        );
    }

    /// Takes in what the background work has reported so far.
    pub fn drain_tasks(&mut self) {
        for event in self.events_rx.try_iter() {
            self.tasks.apply(event);
        }
    }

    pub fn can_save(&self) -> bool {
        self.form.problem().is_none()
    }

    pub fn is_dirty(&self) -> bool {
        self.form.is_changed_from(&self.stored)
    }

    pub fn save(&mut self, services: &Services) {
        if let Some(problem) = self.form.problem() {
            self.notice = Some(Notice::Problem(problem.to_string()));
            return;
        }
        let settings = self.form.to_settings();
        match save_settings(services, &settings) {
            Ok(()) => {
                self.form = SettingsForm::from_settings(&settings);
                self.stored = settings;
                self.notice = Some(Notice::Saved);
            }
            Err(e) => self.notice = Some(Notice::Problem(format!("Could not save: {e}"))),
        }
    }
}

pub struct AppModel {
    pub indicator: IndicatorState,
    /// Last loaded history, shown by the History window.
    pub history: History,
    pub history_open: bool,
    pub settings: Option<SettingsSession>,
    pub quit_requested: bool,
}

impl AppModel {
    pub fn new(now: Instant) -> Self {
        Self {
            indicator: IndicatorState::new(now),
            history: History::default(),
            history_open: false,
            settings: None,
            quit_requested: false,
        }
    }

    /// Takes in every status the engine has published; history is reloaded after each Done.
    pub fn drain_statuses(&mut self, services: &Services, now: Instant) {
        for status in services.engine.status.try_iter() {
            if status == Status::Done {
                self.history = services.history_store.load();
            }
            self.indicator.set_status(status, now);
        }
    }

    pub fn handle_command(&mut self, command: TrayCommand, services: &Services) {
        match command {
            TrayCommand::Settings => {
                if self.settings.is_none() {
                    self.settings = Some(SettingsSession::open(services));
                }
            }
            TrayCommand::History => {
                self.history = services.history_store.load();
                self.history_open = true;
            }
            TrayCommand::Quit => self.quit_requested = true,
        }
    }

    pub fn close_settings(&mut self) {
        self.settings = None;
    }

    pub fn close_history(&mut self) {
        self.history_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wiring::testing::{test_services, Probe};
    use chrono::Local;
    use std::time::Duration;
    use voice_core::{HistoryEntry, Problem, Shortcut};

    fn entry(text: &str) -> HistoryEntry {
        HistoryEntry {
            at: Local::now(),
            raw: format!("raw {text}"),
            polished: text.to_string(),
        }
    }

    fn store_history(services: &Services, texts: &[&str]) {
        let mut history = History::default();
        for t in texts {
            history.record(entry(t));
        }
        services.history_store.save(&history).unwrap();
    }

    fn publish(probe: &Probe, statuses: &[Status]) {
        for s in statuses {
            probe.send_status(s.clone());
        }
    }

    fn wait_for_tasks(session: &mut SettingsSession, done: impl Fn(&TaskState) -> bool) {
        for _ in 0..200 {
            session.drain_tasks();
            if done(&session.tasks) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("background work did not finish: {:?}", session.tasks);
    }

    #[test]
    fn statuses_reach_the_indicator() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        publish(&probe, &[Status::Working]);
        model.drain_statuses(&services, now);
        assert_eq!(model.indicator.view(now).unwrap().text, "Working…");
    }

    #[test]
    fn the_latest_status_wins() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        publish(
            &probe,
            &[
                Status::Listening { level: 0.2 },
                Status::Problem(Problem::NoSpeech),
            ],
        );
        model.drain_statuses(&services, now);
        assert_eq!(model.indicator.view(now).unwrap().text, "No speech heard");
    }

    #[test]
    fn done_reloads_history() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        store_history(&services, &["Hello."]);
        publish(&probe, &[Status::Done]);
        model.drain_statuses(&services, now);
        assert_eq!(model.history.entries().len(), 1);
    }

    #[test]
    fn other_statuses_leave_history_alone() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        store_history(&services, &["Hello."]);
        publish(&probe, &[Status::Working, Status::Cancelled]);
        model.drain_statuses(&services, now);
        assert!(model.history.entries().is_empty());
    }

    #[test]
    fn history_command_reloads_and_opens() {
        let (services, _probe) = test_services();
        let mut model = AppModel::new(Instant::now());
        store_history(&services, &["a", "b"]);
        model.handle_command(TrayCommand::History, &services);
        assert!(model.history_open);
        assert_eq!(model.history.entries().len(), 2);
        model.close_history();
        assert!(!model.history_open);
    }

    #[test]
    fn settings_command_opens_once() {
        let (services, _probe) = test_services();
        let mut model = AppModel::new(Instant::now());
        model.handle_command(TrayCommand::Settings, &services);
        model.settings.as_mut().unwrap().form.assistant_model = "edited".into();
        model.handle_command(TrayCommand::Settings, &services);
        assert_eq!(
            model.settings.as_ref().unwrap().form.assistant_model,
            "edited"
        );
        model.close_settings();
        assert!(model.settings.is_none());
    }

    #[test]
    fn quit_command_requests_quit() {
        let (services, _probe) = test_services();
        let mut model = AppModel::new(Instant::now());
        assert!(!model.quit_requested);
        model.handle_command(TrayCommand::Quit, &services);
        assert!(model.quit_requested);
    }

    #[test]
    fn session_starts_from_stored_settings_and_devices() {
        let (services, _probe) = test_services();
        services
            .settings_store
            .save(&Settings {
                push_to_talk: Shortcut::Fn,
                ..Settings::default()
            })
            .unwrap();
        let session = SettingsSession::open(&services);
        assert_eq!(session.form.push_to_talk, Shortcut::Fn);
        assert_eq!(session.devices, ["Mic A"]);
        assert!(!session.is_dirty());
        assert_eq!(session.notice, None);
    }

    #[test]
    fn unplugged_microphone_stays_selectable() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);
        assert_eq!(session.microphone_options(), ["Mic A"]);
        session.form.microphone = Some("Mic A".into());
        assert_eq!(session.microphone_options(), ["Mic A"]);
        session.form.microphone = Some("Old Mic".into());
        assert_eq!(session.microphone_options(), ["Mic A", "Old Mic"]);
    }

    #[test]
    fn save_persists_applies_and_resets_dirtiness() {
        let (services, probe) = test_services();
        let mut session = SettingsSession::open(&services);
        session.form.cleanup_enabled = false;
        assert!(session.is_dirty());
        session.save(&services);
        assert_eq!(session.notice, Some(Notice::Saved));
        assert!(!session.is_dirty());
        assert!(!services.settings_store.load().cleanup_enabled);
        assert_eq!(probe.engine_settings().len(), 1);
        assert_eq!(probe.shortcut_updates().len(), 1);
    }

    #[test]
    fn save_keeps_a_typed_word_out_of_the_dictionary_until_added() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);
        session.form.new_word = "Kubernetes".into();
        assert!(session.is_dirty());
        session.save(&services);
        assert!(services.settings_store.load().dictionary.is_empty());
    }

    #[test]
    fn invalid_form_is_not_saved() {
        let (services, probe) = test_services();
        let mut session = SettingsSession::open(&services);
        session.form.hands_free = session.form.push_to_talk.clone();
        assert!(!session.can_save());
        session.save(&services);
        assert!(matches!(&session.notice, Some(Notice::Problem(m)) if m.contains("different")));
        assert!(probe.engine_settings().is_empty());
        assert_eq!(services.settings_store.load(), Settings::default());
    }

    #[test]
    fn storage_failure_is_shown_and_keeps_the_form_dirty() {
        let (services, probe) = test_services();
        probe.fail_saving("disk full");
        let mut session = SettingsSession::open(&services);
        session.form.cleanup_enabled = false;
        session.save(&services);
        assert_eq!(
            session.notice,
            Some(Notice::Problem("Could not save: disk full".into()))
        );
        assert!(session.is_dirty());
    }

    #[test]
    fn refreshing_models_lists_them() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);
        session.refresh_models(&services);
        assert_eq!(session.tasks.models, ModelsState::Loading);
        wait_for_tasks(&mut session, |t| t.models != ModelsState::Loading);
        assert_eq!(
            session.tasks.models,
            ModelsState::Loaded(vec!["model-a".into()])
        );
    }

    #[test]
    fn refreshing_twice_while_loading_starts_one_request() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);
        session.tasks.models = ModelsState::Loading;
        session.refresh_models(&services);
        std::thread::sleep(Duration::from_millis(50));
        session.drain_tasks();
        assert_eq!(session.tasks.models, ModelsState::Loading);
    }

    #[test]
    fn download_runs_to_completion() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);
        session.download_model(&services);
        assert!(session.tasks.download.is_running());
        wait_for_tasks(&mut session, |t| !t.download.is_running());
        assert_eq!(
            session.tasks.download,
            crate::tasks::DownloadState::Finished
        );
    }

    #[test]
    fn download_goes_to_the_typed_path_when_given() {
        let (mut services, _probe) = test_services();
        let target = std::sync::Arc::new(std::sync::Mutex::new(None));
        let seen = target.clone();
        services.download_model = std::sync::Arc::new(move |path, _| {
            *seen.lock().unwrap() = Some(path.to_path_buf());
            Ok(())
        });
        let mut session = SettingsSession::open(&services);
        session.form.speech_model_path = "/mine/model.bin".into();
        session.download_model(&services);
        wait_for_tasks(&mut session, |t| !t.download.is_running());
        assert_eq!(
            target.lock().unwrap().as_deref(),
            Some(std::path::Path::new("/mine/model.bin"))
        );
    }

    #[test]
    fn download_to_the_default_location_when_no_path_is_typed() {
        let (mut services, _probe) = test_services();
        let target = std::sync::Arc::new(std::sync::Mutex::new(None));
        let seen = target.clone();
        services.download_model = std::sync::Arc::new(move |path, _| {
            *seen.lock().unwrap() = Some(path.to_path_buf());
            Ok(())
        });
        let mut session = SettingsSession::open(&services);
        session.download_model(&services);
        wait_for_tasks(&mut session, |t| !t.download.is_running());
        assert_eq!(
            target.lock().unwrap().as_deref(),
            Some(std::path::Path::new("/default/model.bin"))
        );
    }

    #[test]
    fn a_second_download_click_while_running_is_ignored() {
        let (mut services, _probe) = test_services();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();
        services.download_model = std::sync::Arc::new(move |_, _| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(100));
            Ok(())
        });
        let mut session = SettingsSession::open(&services);
        session.download_model(&services);
        session.download_model(&services);
        wait_for_tasks(&mut session, |t| !t.download.is_running());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn download_failure_is_shown() {
        let (mut services, _probe) = test_services();
        services.download_model = std::sync::Arc::new(|_, _| Err("offline".into()));
        let mut session = SettingsSession::open(&services);
        session.download_model(&services);
        wait_for_tasks(&mut session, |t| !t.download.is_running());
        assert_eq!(
            session.tasks.download,
            crate::tasks::DownloadState::Failed("offline".into())
        );
    }
}
