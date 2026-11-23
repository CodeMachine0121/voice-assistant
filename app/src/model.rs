//! State of the running app, apart from how it is drawn.
use crate::focus::FocusRequest;
use crate::indicator::IndicatorState;
use crate::permissions_view::PermissionSnapshot;
use crate::settings_form::SettingsForm;
use crate::tasks::{spawn_list_models, ModelsState, TaskEvent, TaskState};
use crate::tray::TrayCommand;
use crate::wiring::{SaveError, Services, SettingsChange};
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::time::Instant;
use voice_core::{History, Settings, Status};
use voice_stt::KeyStatus;

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
    /// Where the OpenAI key comes from. Read when the window opens and after the key
    /// changes, never per frame: every read may reach the Keychain.
    pub key_status: KeyStatus,
    /// What macOS allows, re-read while the window is open.
    pub permissions: PermissionSnapshot,
    events_tx: Sender<TaskEvent>,
    events_rx: Receiver<TaskEvent>,
}

impl SettingsSession {
    pub fn open(services: &Services) -> Self {
        let stored = services.settings_store.load();
        let (events_tx, events_rx) = unbounded();
        let mut session = Self {
            form: SettingsForm::from_settings(&stored),
            stored,
            devices: (services.list_input_devices)(),
            tasks: TaskState::default(),
            notice: None,
            key_status: services.speech_key.status(),
            permissions: PermissionSnapshot::take(&services.permissions, Instant::now()),
            events_tx,
            events_rx,
        };
        // The model lists are asked for as soon as the window opens.
        session.refresh_models(services);
        session
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
            self.notice = Some(Notice::Problem(problem));
            return;
        }
        // A word typed but not yet added would otherwise be silently dropped.
        self.form.add_word();
        let settings = self.form.to_settings();
        let change = SettingsChange {
            settings: settings.clone(),
            new_key: self.form.typed_key().map(str::to_string),
        };
        match (services.save_settings)(&change) {
            Ok(()) => {
                self.key_status = services.speech_key.status();
                self.form = SettingsForm::from_settings(&settings);
                self.stored = settings;
                self.notice = Some(Notice::Saved);
            }
            Err(SaveError::Key(e)) => {
                self.notice = Some(Notice::Problem(format!(
                    "Could not save the OpenAI key: {e}"
                )))
            }
            Err(SaveError::Settings(e)) => {
                self.notice = Some(Notice::Problem(format!("Could not save: {e}")))
            }
        }
    }
}

impl SettingsSession {
    /// Forgets the saved OpenAI key right away (not waiting for Save).
    pub fn remove_key(&mut self, services: &Services) {
        self.notice = match services.speech_key.remove() {
            Ok(()) => {
                self.key_status = services.speech_key.status();
                None
            }
            Err(e) => Some(Notice::Problem(format!(
                "Could not remove the OpenAI key: {e}"
            ))),
        };
    }
}

pub struct AppModel {
    pub indicator: IndicatorState,
    /// Last loaded history, shown by the History window.
    pub history: History,
    pub history_open: bool,
    pub settings: Option<SettingsSession>,
    /// The Settings window should come to the front.
    pub settings_focus: FocusRequest,
    /// The History window should come to the front.
    pub history_focus: FocusRequest,
    pub quit_requested: bool,
}

impl AppModel {
    pub fn new(now: Instant) -> Self {
        Self {
            indicator: IndicatorState::new(now),
            history: History::default(),
            history_open: false,
            settings: None,
            settings_focus: FocusRequest::default(),
            history_focus: FocusRequest::default(),
            quit_requested: false,
        }
    }

    /// Takes in every status the engine has published; history is reloaded after each
    /// finished dictation (Done, or a problem that may still have recorded an entry,
    /// such as cleanup being skipped). `true` when any status arrived.
    pub fn drain_statuses(&mut self, services: &Services, now: Instant) -> bool {
        let mut any = false;
        for status in services.engine.status.try_iter() {
            any = true;
            if matches!(status, Status::Done | Status::Problem(_)) {
                self.history = services.history_store.load();
            }
            self.indicator.set_status(status, now);
        }
        any
    }

    /// Opening a window also brings it to the front with keyboard focus, whether it
    /// is new or already open behind other apps' windows.
    pub fn handle_command(&mut self, command: TrayCommand, services: &Services, now: Instant) {
        match command {
            TrayCommand::Settings => {
                if self.settings.is_none() {
                    self.settings = Some(SettingsSession::open(services));
                }
                (services.app_focus.bring_to_front)();
                self.settings_focus.request(now);
            }
            TrayCommand::History => {
                self.history = services.history_store.load();
                self.history_open = true;
                (services.app_focus.bring_to_front)();
                self.history_focus.request(now);
            }
            TrayCommand::Quit => self.quit_requested = true,
        }
    }

    pub fn close_settings(&mut self, services: &Services) {
        self.settings = None;
        self.settings_focus.cancel();
        self.hand_back_after_last_window(services);
    }

    pub fn close_history(&mut self, services: &Services) {
        self.history_open = false;
        self.history_focus.cancel();
        self.hand_back_after_last_window(services);
    }

    /// Once no window is left, the app the user was in gets focus back, so the next
    /// dictation pastes there and not into this app.
    fn hand_back_after_last_window(&self, services: &Services) {
        if self.settings.is_none() && !self.history_open {
            (services.app_focus.hand_back)();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wiring::testing::{test_services, FocusMove, Probe};
    use chrono::Local;
    use std::sync::Mutex;
    use std::time::Duration;
    use voice_core::{Action, HistoryEntry, Problem, Shortcut};

    fn entry(text: &str) -> HistoryEntry {
        HistoryEntry {
            at: Local::now(),
            action: Action::Dictate,
            original: None,
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
        publish(&probe, &[Status::Working(Action::DictateInEnglish)]);
        model.drain_statuses(&services, now);
        assert_eq!(model.indicator.view(now).unwrap().text, "Translating…");
    }

    #[test]
    fn the_latest_status_wins() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        publish(
            &probe,
            &[
                Status::Listening {
                    action: Action::Dictate,
                    level: 0.2,
                },
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
    fn saving_keeps_a_word_typed_but_not_yet_added() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);
        session.form.new_word = "Kubernetes".to_string();
        session.save(&services);
        assert_eq!(session.notice, Some(Notice::Saved));
        assert_eq!(
            services.settings_store.load().dictionary.words(),
            ["Kubernetes".to_string()]
        );
        assert!(session.form.new_word.is_empty());
    }

    #[test]
    fn a_problem_outcome_also_reloads_history() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        store_history(&services, &["hello there"]);
        publish(
            &probe,
            &[Status::Problem(Problem::AssistantUnreachable(
                Action::Dictate,
            ))],
        );
        model.drain_statuses(&services, now);
        assert_eq!(model.history.entries().len(), 1);
    }

    #[test]
    fn other_statuses_leave_history_alone() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        store_history(&services, &["Hello."]);
        publish(
            &probe,
            &[Status::Working(Action::Dictate), Status::Cancelled],
        );
        model.drain_statuses(&services, now);
        assert!(model.history.entries().is_empty());
    }

    #[test]
    fn history_command_reloads_and_opens() {
        let (services, _probe) = test_services();
        let mut model = AppModel::new(Instant::now());
        store_history(&services, &["a", "b"]);
        model.handle_command(TrayCommand::History, &services, Instant::now());
        assert!(model.history_open);
        assert_eq!(model.history.entries().len(), 2);
        model.close_history(&services);
        assert!(!model.history_open);
    }

    #[test]
    fn settings_command_opens_once() {
        let (services, _probe) = test_services();
        let mut model = AppModel::new(Instant::now());
        model.handle_command(TrayCommand::Settings, &services, Instant::now());
        model.settings.as_mut().unwrap().form.assistant_model = "edited".into();
        model.handle_command(TrayCommand::Settings, &services, Instant::now());
        assert_eq!(
            model.settings.as_ref().unwrap().form.assistant_model,
            "edited"
        );
        model.close_settings(&services);
        assert!(model.settings.is_none());
    }

    #[test]
    fn opening_settings_brings_it_to_the_front() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        model.handle_command(TrayCommand::Settings, &services, now);
        assert_eq!(probe.times_brought_to_front(), 1);
        assert!(model.settings_focus.poll(false, now));
        assert!(!model.history_focus.poll(false, now));
    }

    #[test]
    fn choosing_settings_again_brings_the_open_window_back_to_the_front() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        model.handle_command(TrayCommand::Settings, &services, now);
        assert!(
            !model.settings_focus.poll(true, now),
            "focused: request done"
        );

        model.handle_command(TrayCommand::Settings, &services, now);

        assert_eq!(probe.times_brought_to_front(), 2);
        assert!(model.settings_focus.poll(false, now));
    }

    #[test]
    fn opening_history_brings_it_to_the_front_every_time() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        model.handle_command(TrayCommand::History, &services, now);
        assert!(model.history_focus.poll(false, now));
        assert!(!model.history_focus.poll(true, now));

        model.handle_command(TrayCommand::History, &services, now);

        assert_eq!(probe.times_brought_to_front(), 2);
        assert!(model.history_focus.poll(false, now));
        assert!(!model.settings_focus.poll(false, now));
    }

    #[test]
    fn closing_a_window_stops_asking_for_focus() {
        let (services, _probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        model.handle_command(TrayCommand::Settings, &services, now);
        model.handle_command(TrayCommand::History, &services, now);
        model.close_settings(&services);
        model.close_history(&services);
        assert!(!model.settings_focus.poll(false, now));
        assert!(!model.history_focus.poll(false, now));
    }

    #[test]
    fn quitting_does_not_bring_the_app_to_the_front() {
        let (services, probe) = test_services();
        let mut model = AppModel::new(Instant::now());
        model.handle_command(TrayCommand::Quit, &services, Instant::now());
        assert_eq!(probe.times_brought_to_front(), 0);
    }

    #[test]
    fn closing_settings_while_history_is_open_keeps_focus() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        model.handle_command(TrayCommand::Settings, &services, now);
        model.handle_command(TrayCommand::History, &services, now);

        model.close_settings(&services);

        assert!(!probe.focus_moves().contains(&FocusMove::HandBack));
    }

    #[test]
    fn closing_the_last_window_hands_focus_back() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        model.handle_command(TrayCommand::Settings, &services, now);
        model.handle_command(TrayCommand::History, &services, now);
        model.close_settings(&services);

        model.close_history(&services);

        assert_eq!(
            probe.focus_moves(),
            [
                FocusMove::BringToFront,
                FocusMove::BringToFront,
                FocusMove::HandBack
            ]
        );
    }

    #[test]
    fn closing_history_alone_hands_focus_back() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        model.handle_command(TrayCommand::History, &services, now);

        model.close_history(&services);

        assert_eq!(probe.focus_moves().last(), Some(&FocusMove::HandBack));
    }

    #[test]
    fn statuses_never_move_focus() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        model.handle_command(TrayCommand::Settings, &services, now);
        model.close_settings(&services);

        publish(
            &probe,
            &[Status::Working(Action::Dictate), Status::Cancelled],
        );
        assert!(model.drain_statuses(&services, now));

        assert_eq!(
            probe.focus_moves(),
            [FocusMove::BringToFront, FocusMove::HandBack],
            "dictating after a window closed leaves focus with the app dictated into"
        );
    }

    #[test]
    fn draining_reports_whether_any_status_arrived() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        assert!(!model.drain_statuses(&services, now));
        publish(&probe, &[Status::Working(Action::Dictate)]);
        assert!(model.drain_statuses(&services, now));
        assert!(!model.drain_statuses(&services, now));
    }

    #[test]
    fn opening_and_closing_windows_again_and_again_moves_focus_every_time() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        for _ in 0..3 {
            model.handle_command(TrayCommand::Settings, &services, now);
            model.close_settings(&services);
            model.handle_command(TrayCommand::History, &services, now);
            model.close_history(&services);
        }

        let round = [FocusMove::BringToFront, FocusMove::HandBack];
        assert_eq!(probe.focus_moves(), round.repeat(6));
    }

    #[test]
    fn reopening_after_handing_back_shows_the_window_again() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        model.handle_command(TrayCommand::History, &services, now);
        model.close_history(&services);

        model.handle_command(TrayCommand::History, &services, now);

        assert!(model.history_open);
        assert!(model.history_focus.poll(false, now));
        assert_eq!(probe.times_brought_to_front(), 2);
    }

    #[test]
    fn quit_command_requests_quit() {
        let (services, _probe) = test_services();
        let mut model = AppModel::new(Instant::now());
        assert!(!model.quit_requested);
        model.handle_command(TrayCommand::Quit, &services, Instant::now());
        assert!(model.quit_requested);
    }

    #[test]
    fn session_starts_from_stored_settings_and_devices() {
        let (services, _probe) = test_services();
        services
            .settings_store
            .save(&Settings {
                shortcuts: voice_core::ActionShortcuts {
                    dictate: Shortcut::TapFn,
                    ..Default::default()
                },
                ..Settings::default()
            })
            .unwrap();
        let session = SettingsSession::open(&services);
        assert_eq!(session.form.shortcuts.dictate, Shortcut::TapFn);
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
        assert_eq!(probe.applied_settings().len(), 1);
    }

    #[test]
    fn invalid_form_is_not_saved() {
        let (services, probe) = test_services();
        let mut session = SettingsSession::open(&services);
        session
            .form
            .shortcuts
            .set(Action::EditSelection, Shortcut::TapCommand);
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
    fn opening_settings_lists_the_models_the_local_ai_app_reports() {
        let (mut services, _probe) = test_services();
        services.list_models =
            std::sync::Arc::new(|_| Ok(vec!["breeze-asr-25".into(), "twil-8b".into()]));
        let mut session = SettingsSession::open(&services);
        assert_eq!(session.tasks.models, ModelsState::Loading);
        wait_for_tasks(&mut session, |t| t.models != ModelsState::Loading);
        assert_eq!(
            session.tasks.models,
            ModelsState::Loaded(vec!["breeze-asr-25".into(), "twil-8b".into()])
        );
    }

    #[test]
    fn checking_the_connection_asks_the_typed_address() {
        let (mut services, _probe) = test_services();
        let asked = std::sync::Arc::new(Mutex::new(Vec::new()));
        let seen = asked.clone();
        services.list_models = std::sync::Arc::new(move |url| {
            seen.lock().unwrap().push(url.to_string());
            Err("refused".into())
        });
        let mut session = SettingsSession::open(&services);
        wait_for_tasks(&mut session, |t| t.models != ModelsState::Loading);
        session.form.assistant_url = " http://my-mac:9999/v1 ".into();
        session.refresh_models(&services);
        wait_for_tasks(&mut session, |t| t.models != ModelsState::Loading);
        assert_eq!(
            asked.lock().unwrap().last().map(String::as_str),
            Some("http://my-mac:9999/v1")
        );
        assert_eq!(
            session
                .tasks
                .models
                .connection_line(session.form.assistant_url.trim())
                .as_deref(),
            Some("Local AI app not reachable at http://my-mac:9999/v1")
        );
    }

    #[test]
    fn refreshing_lists_the_models_again() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);
        wait_for_tasks(&mut session, |t| t.models != ModelsState::Loading);
        session.refresh_models(&services);
        assert_eq!(session.tasks.models, ModelsState::Loading);
        wait_for_tasks(&mut session, |t| t.models != ModelsState::Loading);
        assert_eq!(
            session.tasks.models,
            ModelsState::Loaded(vec!["model-a".into()])
        );
    }

    #[test]
    fn saving_a_chosen_speech_model_reaches_the_adapters() {
        let (services, probe) = test_services();
        let mut session = SettingsSession::open(&services);
        session.form.speech.model = "gpt-4o-mini-transcribe".into();

        session.save(&services);

        let applied = probe.applied_settings();
        assert_eq!(applied[0].speech.model, "gpt-4o-mini-transcribe");
    }

    #[test]
    fn saving_a_typed_key_keeps_it_out_of_the_settings_and_clears_the_field() {
        let (services, probe) = test_services();
        let mut session = SettingsSession::open(&services);
        session.form.new_key = "sk-test".into();

        session.save(&services);

        assert_eq!(probe.saved_key().as_deref(), Some("sk-test"));
        assert_eq!(session.key_status, KeyStatus::Saved);
        assert_eq!(session.notice, Some(Notice::Saved));
        assert_eq!(session.form.new_key, "");
        assert!(!session.is_dirty());
        let stored = serde_json::to_string(&services.settings_store.load()).unwrap();
        assert!(!stored.contains("sk-test"), "{stored}");
    }

    #[test]
    fn reopening_settings_never_shows_the_saved_key() {
        let (services, _probe) = test_services();
        services.speech_key.save("sk-test").unwrap();

        let session = SettingsSession::open(&services);

        assert_eq!(session.form.new_key, "");
        assert_eq!(session.key_status, KeyStatus::Saved);
    }

    #[test]
    fn saving_with_the_key_field_empty_keeps_the_saved_key() {
        let (services, probe) = test_services();
        services.speech_key.save("sk-saved").unwrap();
        let mut session = SettingsSession::open(&services);
        session.form.cleanup_enabled = false;

        session.save(&services);

        assert_eq!(probe.saved_key().as_deref(), Some("sk-saved"));
        assert!(!services.settings_store.load().cleanup_enabled);
    }

    #[test]
    fn a_key_that_cannot_be_kept_saves_nothing_else() {
        let (services, probe) = test_services();
        probe.fail_key_storage("keychain locked");
        let mut session = SettingsSession::open(&services);
        session.form.new_key = "sk-test".into();
        session.form.cleanup_enabled = false;

        session.save(&services);

        assert_eq!(
            session.notice,
            Some(Notice::Problem(
                "Could not save the OpenAI key: keychain locked".into()
            ))
        );
        assert!(services.settings_store.load().cleanup_enabled);
        assert!(probe.applied_settings().is_empty());
        assert_eq!(session.form.new_key, "sk-test", "typed key kept for retry");
    }

    #[test]
    fn settings_that_cannot_be_kept_leave_the_saved_key_as_it_was() {
        let (services, probe) = test_services();
        services.speech_key.save("sk-old").unwrap();
        probe.fail_saving("disk full");
        let mut session = SettingsSession::open(&services);
        session.form.new_key = "sk-new".into();

        session.save(&services);

        assert_eq!(
            session.notice,
            Some(Notice::Problem("Could not save: disk full".into()))
        );
        assert_eq!(probe.saved_key().as_deref(), Some("sk-old"));
        assert_eq!(session.form.new_key, "sk-new", "typed key kept for retry");
    }

    #[test]
    fn a_saved_key_that_cannot_be_read_can_still_be_replaced() {
        let (services, probe) = test_services();
        services.speech_key.save("sk-old").unwrap();
        probe.fail_key_reading("the Keychain item is not text");
        let mut session = SettingsSession::open(&services);
        assert_eq!(session.key_status, KeyStatus::Unreadable);
        session.form.new_key = "sk-new".into();

        session.save(&services);

        assert_eq!(session.notice, Some(Notice::Saved));
        assert_eq!(probe.saved_key().as_deref(), Some("sk-new"));
    }

    #[test]
    fn removing_the_key_forgets_it() {
        let (services, probe) = test_services();
        services.speech_key.save("sk-saved").unwrap();
        let mut session = SettingsSession::open(&services);

        session.remove_key(&services);

        assert_eq!(probe.saved_key(), None);
        assert_eq!(session.key_status, KeyStatus::Missing);
        assert_eq!(session.notice, None);
    }

    #[test]
    fn a_key_that_cannot_be_removed_is_reported() {
        let (services, probe) = test_services();
        services.speech_key.save("sk-saved").unwrap();
        probe.fail_key_storage("keychain locked");
        let mut session = SettingsSession::open(&services);

        session.remove_key(&services);

        assert_eq!(
            session.notice,
            Some(Notice::Problem(
                "Could not remove the OpenAI key: keychain locked".into()
            ))
        );
        assert_eq!(probe.saved_key().as_deref(), Some("sk-saved"));
    }

    #[test]
    fn refreshing_twice_while_loading_starts_one_request() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);
        wait_for_tasks(&mut session, |t| t.models != ModelsState::Loading);
        session.tasks.models = ModelsState::Loading;
        session.refresh_models(&services);
        std::thread::sleep(Duration::from_millis(50));
        session.drain_tasks();
        assert_eq!(session.tasks.models, ModelsState::Loading);
    }
}
