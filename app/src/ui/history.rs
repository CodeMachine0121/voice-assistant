//! The History window.
use crate::history_view::{rows, EMPTY_TEXT};
use eframe::egui::{
    self, Frame, RichText, ScrollArea, ViewportBuilder, ViewportCommand, ViewportId,
};
use voice_core::History;

pub fn id() -> ViewportId {
    ViewportId::from_hash_of("history")
}

/// Draws the window; returns `true` when the user closed it.
pub fn show(ctx: &egui::Context, history: &History) -> bool {
    let builder = ViewportBuilder::default()
        .with_title("Dictation History")
        .with_inner_size([520.0, 560.0])
        .with_min_inner_size([360.0, 240.0]);
    let mut closed = false;
    ctx.show_viewport_immediate(id(), builder, |ui, _| {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            closed = true;
        }
        egui::CentralPanel::default().show(ui, |ui| draw(ui, history));
    });
    closed
}

pub fn focus(ctx: &egui::Context) {
    ctx.send_viewport_cmd_to(id(), ViewportCommand::Focus);
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
                        ui.label(RichText::new(&row.time).weak().small());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Copy").clicked() {
                                ui.ctx().copy_text(row.polished.clone());
                            }
                        });
                    });
                    ui.label(RichText::new(&row.polished).size(15.0));
                    if row.raw_differs {
                        ui.label(RichText::new(format!("Heard: {}", row.raw)).weak().small());
                    }
                });
                if index + 1 < rows.len() {
                    ui.add_space(4.0);
                }
            }
        });
}
