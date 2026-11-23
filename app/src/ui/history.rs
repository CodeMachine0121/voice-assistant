//! The History window.
use crate::focus::FocusRequest;
use crate::history_view::{rows, EMPTY_TEXT};
use crate::ui::{action_colour, follow_focus};
use eframe::egui::{
    self, Color32, CornerRadius, Frame, Margin, RichText, ScrollArea, ViewportBuilder, ViewportId,
};
use std::time::Instant;
use voice_core::{Action, History};

pub fn id() -> ViewportId {
    ViewportId::from_hash_of("history")
}

/// Draws the window; returns `true` when the user closed it.
pub fn show(
    ctx: &egui::Context,
    history: &History,
    focus: &mut FocusRequest,
    now: Instant,
) -> bool {
    let builder = ViewportBuilder::default()
        .with_title("Dictation History")
        .with_inner_size([520.0, 560.0])
        .with_min_inner_size([360.0, 240.0]);
    let mut closed = false;
    ctx.show_viewport_immediate(id(), builder, |ui, _| {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            closed = true;
        }
        follow_focus(ui.ctx(), closed, focus, now);
        egui::CentralPanel::default().show(ui, |ui| draw(ui, history));
    });
    closed
}

fn draw(ui: &mut egui::Ui, history: &History) {
    let rows = rows(history);
    if rows.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new(EMPTY_TEXT).weak().size(16.0));
        });
        return;
    }
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, row) in rows.iter().enumerate() {
                Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        chip(ui, row.action, row.chip);
                        ui.label(RichText::new(&row.time).weak().small());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Copy").clicked() {
                                ui.ctx().copy_text(row.result.clone());
                            }
                        });
                    });
                    if let Some(instruction) = &row.instruction {
                        ui.label(RichText::new(instruction).weak());
                    }
                    if let Some(original) = &row.original {
                        ui.label(RichText::new(original).weak().small());
                    }
                    ui.label(RichText::new(&row.result).size(15.0));
                    if let Some(heard) = &row.heard {
                        ui.label(RichText::new(format!("Heard: {heard}")).weak().small());
                    }
                });
                if index + 1 < rows.len() {
                    ui.add_space(4.0);
                }
            }
        });
}

/// A small coloured label naming the action.
fn chip(ui: &mut egui::Ui, action: Action, text: &str) {
    Frame::NONE
        .fill(action_colour(action))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(8, 1))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .small()
                    .strong()
                    .color(Color32::from_rgb(20, 20, 22)),
            );
        });
}
