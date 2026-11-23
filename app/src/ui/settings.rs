//! The Settings window.
use crate::model::{Notice, SettingsSession};
use crate::permissions_view::{self, Access};
use crate::settings_form::{
    assistant_choices, assistant_label, key_status_text, speech_model_choices, SPEECH_PRIVACY_NOTE,
};
use crate::shortcut_names::{caps_text, friendly_name, key_caps, ALL_SHORTCUTS};
use crate::tasks::ModelsState;
use crate::ui::{OK_COLOUR, PROBLEM_COLOUR};
use crate::wiring::Services;
use eframe::egui::{
    self, ComboBox, CornerRadius, Frame, Grid, Margin, RichText, ScrollArea, TextEdit,
    ViewportBuilder, ViewportCommand, ViewportId,
};
use std::time::{Duration, Instant};
use voice_core::{Action, Shortcut, SpeechProvider};
use voice_stt::KeyStatus;

pub fn id() -> ViewportId {
    ViewportId::from_hash_of("settings")
}

/// Draws the window; returns `true` when the user closed it.
pub fn show(ctx: &egui::Context, session: &mut SettingsSession, services: &Services) -> bool {
    let builder = ViewportBuilder::default()
        .with_title("Voice Assistant Settings")
        .with_inner_size([560.0, 720.0])
        .with_min_inner_size([460.0, 400.0]);
    let mut closed = false;
    ctx.show_viewport_immediate(id(), builder, |ui, _| {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            closed = true;
        }
        session.drain_tasks();
        egui::CentralPanel::default().show(ui, |ui| draw(ui, session, services));
        // Permission status changes outside the app; look again now and then.
        ui.ctx().request_repaint_after(Duration::from_secs(1));
    });
    closed
}

pub fn focus(ctx: &egui::Context) {
    ctx.send_viewport_cmd_to(id(), ViewportCommand::Focus);
}

fn draw(ui: &mut egui::Ui, session: &mut SettingsSession, services: &Services) {
    egui::Panel::bottom("settings_footer").show(ui, |ui| footer(ui, session, services));
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            section(ui, "Shortcuts", |ui| shortcuts(ui, session));
            section(ui, "AI models", |ui| ai_models(ui, session, services));
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

fn shortcuts(ui: &mut egui::Ui, session: &mut SettingsSession) {
    for action in Action::ALL {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(action.label()).strong());
                ui.label(RichText::new(action.description()).weak().small());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut value = *session.form.shortcuts.for_action(action);
                shortcut_picker(ui, action, &mut value);
                session.form.shortcuts.set(action, value);
            });
        });
        if let Some(conflict) = session.form.shortcut_conflict(action) {
            ui.colored_label(PROBLEM_COLOUR, conflict);
        }
        ui.add_space(4.0);
    }
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

fn shortcut_picker(ui: &mut egui::Ui, action: Action, value: &mut Shortcut) {
    ComboBox::from_id_salt(("shortcut", action.label()))
        .width(200.0)
        .selected_text(caps_text(value))
        .show_ui(ui, |ui| {
            for shortcut in ALL_SHORTCUTS {
                ui.horizontal(|ui| {
                    for key in key_caps(&shortcut) {
                        key_cap(ui, key);
                    }
                    if ui
                        .selectable_label(*value == shortcut, friendly_name(&shortcut))
                        .clicked()
                    {
                        *value = shortcut;
                    }
                });
            }
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

fn ai_models(ui: &mut egui::Ui, session: &mut SettingsSession, services: &Services) {
    speech_rows(ui, session, services);
    ui.add_space(12.0);

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

    ui.checkbox(&mut session.form.cleanup_enabled, "AI cleanup");
    ui.label(RichText::new("Applies to Dictate only").weak().small());
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
    choice_row(ui, "Provider", "turns your voice into words", |ui| {
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
        "the provider's model that listens",
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
        if status == KeyStatus::Saved {
            remove = ui.button("Remove key").clicked();
        }
        ui.add(
            TextEdit::singleline(&mut session.form.new_key)
                .password(true)
                .hint_text(match status {
                    KeyStatus::Saved => "Type to replace",
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
        "Writing assistant model",
        "cleans up, translates and edits, in the local AI app",
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
    fn a_typed_key_is_drawn_hidden() {
        let (services, _probe) = test_services();
        let mut session = SettingsSession::open(&services);
        session.form.new_key = "sk-typed".into();

        let drawn = drawn_speech_rows(&mut session, &services);

        assert!(!drawn.iter().any(|t| t.contains("sk-typed")), "{drawn:?}");
    }
}
