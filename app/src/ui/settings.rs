//! The Settings window.
use crate::model::{Notice, SettingsSession};
use crate::permissions_view::{self, Access};
use crate::shortcut_names::{friendly_name, ALL_SHORTCUTS};
use crate::wiring::Services;
use eframe::egui::{
    self, Color32, ComboBox, Grid, ProgressBar, RichText, ScrollArea, TextEdit, ViewportBuilder,
    ViewportCommand, ViewportId,
};
use std::time::{Duration, Instant};

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
            section(ui, "Microphone", |ui| microphone(ui, session));
            section(ui, "Writing assistant", |ui| {
                assistant(ui, session, services)
            });
            section(ui, "Dictionary", |ui| dictionary(ui, session));
            section(ui, "Speech model", |ui| speech_model(ui, session, services));
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
    Grid::new("shortcuts")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("Hold to talk");
            shortcut_picker(ui, "push_to_talk", &mut session.form.push_to_talk);
            ui.end_row();
            ui.label("Hands-free");
            shortcut_picker(ui, "hands_free", &mut session.form.hands_free);
            ui.end_row();
        });
    if let Some(problem) = session.form.problem().filter(|p| p.contains("shortcuts")) {
        ui.colored_label(Color32::from_rgb(255, 99, 88), problem);
    }
}

fn shortcut_picker(ui: &mut egui::Ui, salt: &str, value: &mut voice_core::Shortcut) {
    ComboBox::from_id_salt(salt)
        .width(220.0)
        .selected_text(friendly_name(value))
        .show_ui(ui, |ui| {
            for shortcut in ALL_SHORTCUTS {
                let name = friendly_name(&shortcut);
                ui.selectable_value(value, shortcut, name);
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

fn assistant(ui: &mut egui::Ui, session: &mut SettingsSession, services: &Services) {
    ui.checkbox(&mut session.form.cleanup_enabled, "AI cleanup");
    ui.add_enabled_ui(session.form.cleanup_enabled, |ui| {
        Grid::new("assistant")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Address");
                ui.add(
                    TextEdit::singleline(&mut session.form.assistant_url)
                        .hint_text("http://localhost:1234/v1")
                        .desired_width(300.0),
                );
                ui.end_row();
                ui.label("Model");
                ui.horizontal(|ui| {
                    ui.add(
                        TextEdit::singleline(&mut session.form.assistant_model)
                            .hint_text("whatever is loaded")
                            .desired_width(220.0),
                    );
                    if ui.button("Refresh").clicked() {
                        session.refresh_models(services);
                    }
                });
                ui.end_row();
            });
        let models = session.tasks.models.models().to_vec();
        if !models.is_empty() {
            ComboBox::from_id_salt("models")
                .width(300.0)
                .selected_text("Pick a listed model")
                .show_ui(ui, |ui| {
                    for model in models {
                        ui.selectable_value(
                            &mut session.form.assistant_model,
                            model.clone(),
                            model,
                        );
                    }
                });
        }
        if let Some(message) = session.tasks.models.message() {
            ui.label(RichText::new(message).weak().small());
        }
    });
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

fn speech_model(ui: &mut egui::Ui, session: &mut SettingsSession, services: &Services) {
    let default_path = (services.default_model_path)();
    ui.add(
        TextEdit::singleline(&mut session.form.speech_model_path)
            .hint_text(default_path.display().to_string())
            .desired_width(f32::INFINITY),
    );
    ui.label(
        RichText::new("Leave empty to use the default location.")
            .weak()
            .small(),
    );
    ui.horizontal(|ui| {
        let running = session.tasks.download.is_running();
        if ui
            .add_enabled(!running, egui::Button::new("Download default model"))
            .clicked()
        {
            session.download_model(services);
        }
    });
    if let Some(fraction) = session.tasks.download.fraction() {
        ui.add(ProgressBar::new(fraction).show_percentage());
    } else if session.tasks.download.is_running() {
        ui.add(ProgressBar::new(0.0).animate(true));
    }
    if let Some(message) = session.tasks.download.message() {
        ui.label(RichText::new(message).weak().small());
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
                    Access::Granted => Color32::from_rgb(52, 199, 89),
                    Access::Denied => Color32::from_rgb(255, 99, 88),
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
                ui.colored_label(Color32::from_rgb(52, 199, 89), "Saved");
            }
            Some(Notice::Problem(message)) => {
                ui.colored_label(Color32::from_rgb(255, 99, 88), message);
            }
            _ => {
                if let Some(problem) = session.form.problem() {
                    ui.colored_label(Color32::from_rgb(255, 99, 88), problem);
                }
            }
        }
    });
    ui.add_space(6.0);
}
