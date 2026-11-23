//! The Settings window.
use crate::focus::FocusRequest;
use crate::model::{Notice, SettingsSession};
use crate::permissions_view::{self, Access};
use crate::settings_form::{
    assistant_choices, assistant_label, key_status_text, speech_model_choices, writing_use,
    CLEANUP_LABEL, SPEECH_GROUP, SPEECH_PRIVACY_NOTE, SPEECH_PURPOSE, WRITING_GROUP,
    WRITING_PURPOSE,
};
use crate::shortcut_names::key_caps;
use crate::tasks::ModelsState;
use crate::ui::{follow_focus, OK_COLOUR, PROBLEM_COLOUR};
use crate::wiring::Services;
use eframe::egui::{
    self, ComboBox, CornerRadius, Frame, Grid, Margin, RichText, ScrollArea, TextEdit,
    ViewportBuilder, ViewportId,
};
use std::time::{Duration, Instant};
use voice_core::{Action, Shortcut, SpeechProvider};
use voice_stt::KeyStatus;

pub fn id() -> ViewportId {
    ViewportId::from_hash_of("settings")
}

/// Draws the window; returns `true` when the user closed it.
pub fn show(
    ctx: &egui::Context,
    session: &mut SettingsSession,
    services: &Services,
    focus: &mut FocusRequest,
    now: Instant,
) -> bool {
    let builder = ViewportBuilder::default()
        .with_title("Voice Assistant Settings")
        .with_inner_size([560.0, 720.0])
        .with_min_inner_size([460.0, 400.0]);
    let mut closed = false;
    ctx.show_viewport_immediate(id(), builder, |ui, _| {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            closed = true;
        }
        follow_focus(ui.ctx(), closed, focus, now);
        session.drain_tasks();
        egui::CentralPanel::default().show(ui, |ui| draw(ui, session, services));
        // Permission status changes outside the app; look again now and then.
        ui.ctx().request_repaint_after(Duration::from_secs(1));
    });
    closed
}

fn draw(ui: &mut egui::Ui, session: &mut SettingsSession, services: &Services) {
    egui::Panel::bottom("settings_footer").show(ui, |ui| footer(ui, session, services));
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            section(ui, "Shortcuts", shortcuts);
            section(ui, SPEECH_GROUP, |ui| speech_to_text(ui, session, services));
            section(ui, WRITING_GROUP, |ui| {
                writing_assistant(ui, session, services)
            });
            section(ui, "Microphone", |ui| microphone(ui, session));
            section(ui, "Dictionary", |ui| dictionary(ui, session));
            section(ui, "Permissions", |ui| permissions(ui, session, services));
        });
}

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(8.0);
    ui.heading(title);
    ui.add_space(4.0);
    body(ui);
    ui.add_space(6.0);
    ui.separator();
}

/// Each action with its fixed shortcut. Only the right-hand modifier keys count.
fn shortcuts(ui: &mut egui::Ui) {
    for action in Action::ALL {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(action.label()).strong());
                ui.label(RichText::new(action.description()).weak().small());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Right to left: the last key first.
                for (i, key) in shortcut_keys(action).iter().rev().enumerate() {
                    if i > 0 {
                        ui.label(RichText::new("+").weak());
                    }
                    key_cap(ui, key);
                }
            });
        });
        ui.add_space(4.0);
    }
    ui.label(RichText::new(SHORTCUTS_NOTE).weak().small());
}

/// Under the shortcuts: why the left-hand keys do nothing.
pub const SHORTCUTS_NOTE: &str =
    "Only the right-hand ⌘, ⇧ and ⌥ keys count, so the left ones keep working as usual.";

/// The key caps of the shortcut of `action`.
fn shortcut_keys(action: Action) -> &'static [&'static str] {
    key_caps(Shortcut::for_action(action))
}

/// A key cap: one key of a shortcut drawn as a small rounded box.
fn key_cap(ui: &mut egui::Ui, key: &str) {
    Frame::NONE
        .fill(ui.visuals().widgets.inactive.bg_fill)
        .stroke(ui.visuals().widgets.inactive.bg_stroke)
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(5, 0))
        .show(ui, |ui| {
            ui.label(RichText::new(key).monospace().small());
        });
}

fn microphone(ui: &mut egui::Ui, session: &mut SettingsSession) {
    let options = session.microphone_options();
    let selected = session
        .form
        .microphone
        .clone()
        .unwrap_or_else(|| "System default".to_string());
    ComboBox::from_id_salt("microphone")
        .width(300.0)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut session.form.microphone, None, "System default");
            for name in options {
                ui.selectable_value(&mut session.form.microphone, Some(name.clone()), name);
            }
        });
}

/// A group's one-line purpose, under its title.
fn purpose(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).weak());
    ui.add_space(6.0);
}

/// The OpenAI model that turns the recording into words.
fn speech_to_text(ui: &mut egui::Ui, session: &mut SettingsSession, services: &Services) {
    purpose(ui, SPEECH_PURPOSE);
    speech_rows(ui, session, services);
}

/// The model in the local AI app that works on the words, and what each action uses it for.
fn writing_assistant(ui: &mut egui::Ui, session: &mut SettingsSession, services: &Services) {
    purpose(ui, WRITING_PURPOSE);
    writing_uses(ui, session);
    ui.add_space(8.0);
    writing_rows(ui, session, services);
}

/// One line per action: its name, its shortcut and what the writing model does for it.
fn writing_uses(ui: &mut egui::Ui, session: &SettingsSession) {
    for action in Action::ALL {
        ui.horizontal(|ui| {
            ui.label(RichText::new(action.label()).strong());
            for key in shortcut_keys(action) {
                key_cap(ui, key);
            }
            ui.label(
                RichText::new(writing_use(action, session.form.cleanup_enabled))
                    .weak()
                    .small(),
            );
        });
    }
}

/// Local AI app address, writing model and the AI cleanup switch.
fn writing_rows(ui: &mut egui::Ui, session: &mut SettingsSession, services: &Services) {
    ui.horizontal(|ui| {
        ui.label("Local AI app address");
        ui.add(
            TextEdit::singleline(&mut session.form.assistant_url)
                .hint_text("http://localhost:1234/v1")
                .desired_width(260.0),
        );
        let checking = session.tasks.models == ModelsState::Loading;
        if ui
            .add_enabled(!checking, egui::Button::new("Check connection"))
            .clicked()
        {
            session.refresh_models(services);
        }
    });
    let address = session.form.assistant_url.trim().to_string();
    if let Some(line) = session.tasks.models.connection_line(&address) {
        let colour = match session.tasks.models {
            ModelsState::Failed(_) => PROBLEM_COLOUR,
            ModelsState::Loaded(_) => OK_COLOUR,
            _ => ui.visuals().weak_text_color(),
        };
        ui.colored_label(colour, line);
    }
    if let Some(warning) = session.form.address_warning() {
        ui.colored_label(PROBLEM_COLOUR, warning);
    }
    ui.add_space(8.0);

    assistant_row(ui, session, services);
    ui.add_space(8.0);

    ui.checkbox(&mut session.form.cleanup_enabled, CLEANUP_LABEL);
}

/// Label, description and a right-aligned choice with a refresh button.
fn model_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    add_choice: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut refresh = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).strong());
            ui.label(RichText::new(description).weak().small());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            refresh = ui.button("⟳").on_hover_text("Refresh").clicked();
            add_choice(ui);
        });
    });
    refresh
}

/// Label and description on the left, `add_choice` right-aligned.
fn choice_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    add_choice: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).strong());
            ui.label(RichText::new(description).weak().small());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add_choice);
    });
}

/// Speech provider, its speech model and key, and where recordings go.
fn speech_rows(ui: &mut egui::Ui, session: &mut SettingsSession, services: &Services) {
    let speech = &mut session.form.speech;
    choice_row(ui, "Provider", "the service that listens", |ui| {
        ComboBox::from_id_salt("speech_provider")
            .width(220.0)
            .selected_text(speech.provider.label())
            .show_ui(ui, |ui| {
                for provider in SpeechProvider::ALL {
                    ui.selectable_value(&mut speech.provider, provider, provider.label());
                }
            });
    });
    ui.add_space(4.0);
    let choices = speech_model_choices(speech.provider, &speech.model);
    choice_row(
        ui,
        "Speech model",
        "the provider's speech-to-text model",
        |ui| {
            ComboBox::from_id_salt("speech_model")
                .width(220.0)
                .selected_text(speech.model.clone())
                .show_ui(ui, |ui| {
                    for choice in choices {
                        let label = choice.clone();
                        ui.selectable_value(&mut speech.model, choice, label);
                    }
                });
        },
    );
    ui.add_space(4.0);
    let status = session.key_status;
    let mut remove = false;
    choice_row(ui, "OpenAI key", key_status_text(status), |ui| {
        // A key that cannot be read can still be removed, so the user is never stuck.
        if matches!(status, KeyStatus::Saved | KeyStatus::Unreadable) {
            remove = ui.button("Remove key").clicked();
        }
        ui.add(
            TextEdit::singleline(&mut session.form.new_key)
                .password(true)
                .hint_text(match status {
                    KeyStatus::Saved | KeyStatus::Unreadable => "Type to replace",
                    _ => "sk-…",
                })
                .desired_width(220.0),
        );
    });
    if remove {
        session.remove_key(services);
    }
    ui.label(RichText::new(SPEECH_PRIVACY_NOTE).weak().small());
}

fn assistant_row(ui: &mut egui::Ui, session: &mut SettingsSession, services: &Services) {
    let listed = session.tasks.models.models().to_vec();
    let choices = assistant_choices(&listed, &session.form.assistant_model);
    let refresh = model_row(
        ui,
        "Writing model",
        "the model loaded in the local AI app",
        |ui| {
            ComboBox::from_id_salt("assistant_model")
                .width(220.0)
                .selected_text(assistant_label(&session.form.assistant_model))
                .show_ui(ui, |ui| {
                    for choice in choices {
                        let label = assistant_label(&choice);
                        ui.selectable_value(&mut session.form.assistant_model, choice, label);
                    }
                });
        },
    );
    if refresh {
        session.refresh_models(services);
    }
    if let Some(note) = session.tasks.models.list_note() {
        ui.label(RichText::new(note).weak().small());
    }
}

fn dictionary(ui: &mut egui::Ui, session: &mut SettingsSession) {
    ui.label(
        RichText::new("Words that must always be spelled exactly.")
            .weak()
            .small(),
    );
    let mut add = false;
    ui.horizontal(|ui| {
        let field = ui.add(
            TextEdit::singleline(&mut session.form.new_word)
                .hint_text("Add a word")
                .desired_width(240.0),
        );
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        add = ui.button("Add").clicked() || enter;
        if enter {
            field.request_focus();
        }
    });
    if add {
        session.form.add_word();
    }
    let mut remove = None;
    for word in session.form.words() {
        ui.horizontal(|ui| {
            ui.label(word);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Remove").clicked() {
                    remove = Some(word.clone());
                }
            });
        });
    }
    if let Some(word) = remove {
        session.form.remove_word(&word);
    }
}

fn permissions(ui: &mut egui::Ui, session: &mut SettingsSession, services: &Services) {
    let rows = session
        .permissions
        .current(&services.permissions, Instant::now())
        .to_vec();
    Grid::new("permissions")
        .num_columns(3)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            for row in rows {
                ui.vertical(|ui| {
                    ui.label(row.name);
                    ui.label(RichText::new(row.why).weak().small());
                });
                let colour = match row.access {
                    Access::Granted => OK_COLOUR,
                    Access::Denied => PROBLEM_COLOUR,
                    Access::NotAsked | Access::Unknown => ui.visuals().weak_text_color(),
                };
                ui.colored_label(colour, row.status_text());
                if ui.button(row.button_text()).clicked() {
                    permissions_view::open_settings(&services.permissions, &row);
                }
                ui.end_row();
            }
        });
}

fn footer(ui: &mut egui::Ui, session: &mut SettingsSession, services: &Services) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let enabled = session.can_save() && session.is_dirty();
        if ui.add_enabled(enabled, egui::Button::new("Save")).clicked() {
            session.save(services);
        }
        match &session.notice {
            Some(Notice::Saved) if !session.is_dirty() => {
                ui.colored_label(OK_COLOUR, "Saved");
            }
            Some(Notice::Problem(message)) => {
                ui.colored_label(PROBLEM_COLOUR, message);
            }
            // A shortcut clash is already shown under its row.
            _ => {
                if let Some(problem) = session.form.address_problem() {
                    ui.colored_label(PROBLEM_COLOUR, problem);
                }
            }
        }
    });
    ui.add_space(6.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wiring::testing::test_services;
    use eframe::egui::epaint::Shape;

    fn texts(shape: &Shape, out: &mut Vec<String>) {
        match shape {
            Shape::Text(text) => out.push(text.galley.text().to_string()),
            Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }

    /// Everything the speech rows draw as text in one frame.
    fn drawn_speech_rows(session: &mut SettingsSession, services: &Services) -> Vec<String> {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            speech_rows(ui, session, services);
        });
        // No renderer takes the font textures in a test.
        output.textures_delta.clear();
        let mut out = Vec::new();
        for clipped in &output.shapes {
            texts(&clipped.shape, &mut out);
        }
        out
    }

    #[test]
    fn speech_rows_show_openai_its_model_and_where_recordings_go() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);

        let drawn = drawn_speech_rows(&mut session, &services);

        for expected in [
            "Provider",
            "OpenAI",
            "Speech model",
            "gpt-4o-transcribe",
            "OpenAI key",
            "No OpenAI key saved",
            "Recordings are sent to OpenAI to be turned into words.",
        ] {
            assert!(drawn.iter().any(|t| t == expected), "{expected}: {drawn:?}");
        }
        assert!(!drawn.iter().any(|t| t == "Remove key"), "{drawn:?}");
    }

    #[test]
    fn a_saved_key_can_be_removed_and_is_never_drawn() {
        let (services, _probe) = test_services();
        services.speech_key.save("sk-secret").unwrap();
        let mut session = SettingsSession::open(&services);

        let drawn = drawn_speech_rows(&mut session, &services);

        assert!(drawn.iter().any(|t| t == "OpenAI key saved"), "{drawn:?}");
        assert!(drawn.iter().any(|t| t == "Remove key"), "{drawn:?}");
        assert!(!drawn.iter().any(|t| t.contains("sk-secret")), "{drawn:?}");
    }

    /// Everything the whole window draws as text in one frame, top to bottom.
    fn drawn_window(session: &mut SettingsSession, services: &Services) -> Vec<String> {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(560.0, 4_000.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| draw(ui, session, services));
        output.textures_delta.clear();
        let mut drawn = Vec::new();
        for clipped in &output.shapes {
            texts(&clipped.shape, &mut drawn);
        }
        drawn
    }

    fn position(drawn: &[String], text: &str) -> usize {
        drawn
            .iter()
            .position(|t| t == text)
            .unwrap_or_else(|| panic!("{text} not drawn: {drawn:?}"))
    }

    #[test]
    fn the_shortcuts_group_shows_the_three_fixed_right_hand_shortcuts() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);

        let drawn = drawn_window(&mut session, &services);

        let start = position(&drawn, "Shortcuts") + 1;
        let end = position(&drawn, SPEECH_GROUP);
        let d = |a: Action| a.description().to_string();
        let expected: Vec<String> = [
            "Dictate".to_string(),
            d(Action::Dictate),
            "Right ⌘".into(),
            "English".into(),
            d(Action::DictateInEnglish),
            // Drawn right to left: the last key first.
            "Right ⇧".into(),
            "+".into(),
            "Right ⌘".into(),
            "Edit".into(),
            d(Action::EditSelection),
            "Space".into(),
            "+".into(),
            "Right ⌥".into(),
            SHORTCUTS_NOTE.into(),
        ]
        .into();
        assert_eq!(
            drawn[start..end],
            expected,
            "nothing to pick, just the keys"
        );
    }

    #[test]
    fn speech_and_writing_models_are_in_separate_groups() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);

        let drawn = drawn_window(&mut session, &services);

        let speech = position(&drawn, SPEECH_GROUP);
        let writing = position(&drawn, WRITING_GROUP);
        let next = position(&drawn, "Microphone");
        assert!(speech < writing && writing < next, "{drawn:?}");
        for row in [
            SPEECH_PURPOSE,
            "Provider",
            "Speech model",
            "OpenAI key",
            SPEECH_PRIVACY_NOTE,
        ] {
            let at = position(&drawn, row);
            assert!(speech < at && at < writing, "{row} is not under speech");
        }
        for row in [
            WRITING_PURPOSE,
            "Local AI app address",
            "Check connection",
            "Writing model",
            CLEANUP_LABEL,
        ] {
            let at = position(&drawn, row);
            assert!(writing < at && at < next, "{row} is not under writing");
        }
        assert!(!drawn.iter().any(|t| t.contains("AI models")), "{drawn:?}");
    }

    #[test]
    fn the_writing_group_lists_what_each_action_uses_it_for() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);
        session.form.cleanup_enabled = true;

        let drawn = drawn_window(&mut session, &services);

        let writing = position(&drawn, WRITING_GROUP);
        let under_writing = &drawn[writing..];
        for action in Action::ALL {
            let name = position(under_writing, action.label());
            let caps = shortcut_keys(action);
            let used_for = position(under_writing, writing_use(action, true));
            assert!(name < used_for, "{action:?}: {under_writing:?}");
            for (offset, key) in caps.iter().enumerate() {
                assert_eq!(&under_writing[name + 1 + offset], key, "{action:?}");
            }
        }
    }

    #[test]
    fn turning_ai_cleanup_off_shows_dictate_does_not_use_the_writing_model() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);
        session.form.cleanup_enabled = false;

        let drawn = drawn_window(&mut session, &services);

        position(&drawn, writing_use(Action::Dictate, false));
        assert!(!drawn
            .iter()
            .any(|t| t == writing_use(Action::Dictate, true)));
    }

    #[test]
    fn the_settings_window_no_longer_offers_a_built_in_speech_model() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);

        let drawn = drawn_window(&mut session, &services);

        // The whole window was drawn, from the speech rows down to the permissions.
        for expected in ["OpenAI", "Writing model", "Permissions", "Save"] {
            assert!(drawn.iter().any(|t| t == expected), "{expected}: {drawn:?}");
        }
        for gone in ["Built-in", "Download", "model file", "default location"] {
            assert!(!drawn.iter().any(|t| t.contains(gone)), "{gone}: {drawn:?}");
        }
    }

    #[test]
    fn drawing_frames_does_not_read_the_key_store_again() {
        let (services, probe) = test_services();
        let mut session = SettingsSession::open(&services);
        let reads_on_open = probe.key_reads();

        for _ in 0..5 {
            drawn_speech_rows(&mut session, &services);
        }

        assert_eq!(reads_on_open, 1);
        assert_eq!(probe.key_reads(), 1);
    }

    #[test]
    fn a_key_that_cannot_be_read_is_reported_and_can_be_removed() {
        let (services, probe) = test_services();
        services.speech_key.save("sk-secret").unwrap();
        probe.fail_key_reading("access denied");
        let mut session = SettingsSession::open(&services);

        let drawn = drawn_speech_rows(&mut session, &services);

        assert!(
            drawn
                .iter()
                .any(|t| t == "Couldn't read the OpenAI key from the Keychain"),
            "{drawn:?}"
        );
        assert!(drawn.iter().any(|t| t == "Remove key"), "{drawn:?}");
    }

    #[test]
    fn a_typed_key_is_drawn_hidden() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);
        session.form.new_key = "sk-typed".into();

        let drawn = drawn_speech_rows(&mut session, &services);

        assert!(!drawn.iter().any(|t| t.contains("sk-typed")), "{drawn:?}");
    }
}
