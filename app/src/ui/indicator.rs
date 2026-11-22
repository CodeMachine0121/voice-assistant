//! The floating indicator: a borderless, transparent, always-on-top pill near the
//! bottom of the screen that never takes focus or mouse clicks.
use crate::indicator::{indicator_position, IndicatorKind, IndicatorView, INDICATOR_SIZE};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Frame, Rect, Sense, Spinner, Stroke, StrokeKind,
    Vec2, ViewportBuilder, ViewportId,
};

const FALLBACK_MONITOR: Vec2 = Vec2::new(1440.0, 900.0);

pub fn show(ctx: &egui::Context, view: &IndicatorView) {
    let monitor = ctx
        .input(|i| i.viewport().monitor_size)
        .unwrap_or(FALLBACK_MONITOR);
    let builder = ViewportBuilder::default()
        .with_title("Voice Assistant")
        .with_inner_size(INDICATOR_SIZE)
        .with_position(indicator_position(monitor, INDICATOR_SIZE))
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_mouse_passthrough(true)
        .with_active(false)
        .with_taskbar(false)
        .with_resizable(false)
        .with_has_shadow(false);
    ctx.show_viewport_immediate(ViewportId::from_hash_of("indicator"), builder, |ui, _| {
        egui::CentralPanel::default()
            .frame(Frame::NONE)
            .show(ui, |ui| draw(ui, view));
    });
}

fn accent(kind: IndicatorKind) -> Color32 {
    match kind {
        IndicatorKind::Listening => Color32::from_rgb(235, 64, 52),
        IndicatorKind::Working => Color32::from_rgb(245, 166, 35),
        IndicatorKind::Done => Color32::from_rgb(52, 199, 89),
        IndicatorKind::Cancelled => Color32::from_rgb(160, 160, 166),
        IndicatorKind::Problem => Color32::from_rgb(255, 99, 88),
    }
}

fn draw(ui: &mut egui::Ui, view: &IndicatorView) {
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let painter = ui.painter_at(rect);
    let pill = rect.shrink(2.0);
    let radius = CornerRadius::same((pill.height() / 2.0) as u8);
    painter.rect_filled(
        pill,
        radius,
        Color32::from_rgba_unmultiplied(28, 28, 30, 235),
    );
    painter.rect_stroke(
        pill,
        radius,
        Stroke::new(1.0, Color32::from_white_alpha(30)),
        StrokeKind::Inside,
    );

    let colour = accent(view.kind);
    let left = pill.left() + 18.0;
    let centre_y = pill.center().y;
    match view.kind {
        IndicatorKind::Working => {
            let spinner_rect =
                Rect::from_center_size(egui::pos2(left + 8.0, centre_y), Vec2::splat(18.0));
            ui.put(spinner_rect, Spinner::new().color(colour));
        }
        _ => {
            painter.circle_filled(egui::pos2(left + 8.0, centre_y), 6.0, colour);
        }
    }

    let text_left = left + 28.0;
    let has_bar = view.kind == IndicatorKind::Listening;
    let bar_width = 96.0;
    let text_right = if has_bar {
        pill.right() - 18.0 - bar_width - 12.0
    } else {
        pill.right() - 18.0
    };
    painter.text(
        egui::pos2(text_left, centre_y),
        Align2::LEFT_CENTER,
        fit(&view.text, ((text_right - text_left) / 7.0) as usize),
        FontId::proportional(14.0),
        Color32::WHITE,
    );

    if has_bar {
        let track = Rect::from_min_size(
            egui::pos2(pill.right() - 18.0 - bar_width, centre_y - 3.0),
            Vec2::new(bar_width, 6.0),
        );
        painter.rect_filled(track, CornerRadius::same(3), Color32::from_white_alpha(40));
        let fill = Rect::from_min_size(track.min, Vec2::new(bar_width * view.level, 6.0));
        painter.rect_filled(fill, CornerRadius::same(3), colour);
    }
}

/// Shortens `text` with an ellipsis so it fits roughly `max_chars` characters.
fn fit(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars || max_chars < 2 {
        text.to_string()
    } else {
        let mut short: String = text.chars().take(max_chars - 1).collect();
        short.push('…');
        short
    }
}
