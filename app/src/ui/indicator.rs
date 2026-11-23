//! The floating indicator: a borderless, transparent, always-on-top pill near the
//! bottom of the screen that never takes focus or mouse clicks.
use crate::indicator::{indicator_position, IndicatorKind, IndicatorView, INDICATOR_SIZE};
use crate::ui::action_colour;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Frame, Rect, Sense, Spinner, Stroke, StrokeKind,
    Vec2, ViewportBuilder, ViewportId,
};
use voice_core::Action;

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

    if let (IndicatorKind::Listening, Some(action)) = (view.kind, view.action) {
        draw_listening(&painter, pill, view, action);
    } else {
        draw_message(ui, &painter, pill, view);
    }
}

fn draw_listening(painter: &egui::Painter, pill: Rect, view: &IndicatorView, action: Action) {
    let centre_y = pill.center().y;
    let colour = action_colour(action);

    // Action chip on the left.
    let chip_font = FontId::proportional(13.0);
    let label = painter.layout_no_wrap(
        action.label().to_string(),
        chip_font,
        Color32::from_rgb(20, 20, 22),
    );
    let chip = Rect::from_min_size(
        egui::pos2(pill.left() + 14.0, centre_y - 12.0),
        Vec2::new(label.size().x + 20.0, 24.0),
    );
    painter.rect_filled(chip, CornerRadius::same(12), colour);
    painter.galley(
        egui::pos2(chip.left() + 10.0, centre_y - label.size().y / 2.0),
        label,
        Color32::BLACK,
    );

    // Elapsed time on the right.
    let elapsed = view.elapsed.clone().unwrap_or_default();
    let time_right = pill.right() - 18.0;
    let time = painter.text(
        egui::pos2(time_right, centre_y),
        Align2::RIGHT_CENTER,
        elapsed,
        FontId::monospace(14.0),
        Color32::WHITE,
    );

    // Level bar fills the space between.
    let bar_left = chip.right() + 14.0;
    let bar_right = time.left() - 14.0;
    if bar_right > bar_left {
        let width = bar_right - bar_left;
        let track =
            Rect::from_min_size(egui::pos2(bar_left, centre_y - 3.0), Vec2::new(width, 6.0));
        painter.rect_filled(track, CornerRadius::same(3), Color32::from_white_alpha(40));
        let fill = Rect::from_min_size(track.min, Vec2::new(width * view.level, 6.0));
        painter.rect_filled(fill, CornerRadius::same(3), colour);
    }
}

fn draw_message(ui: &mut egui::Ui, painter: &egui::Painter, pill: Rect, view: &IndicatorView) {
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
    let text_right = pill.right() - 18.0;
    painter.text(
        egui::pos2(text_left, centre_y),
        Align2::LEFT_CENTER,
        fit(&view.text, ((text_right - text_left) / 7.0) as usize),
        FontId::proportional(14.0),
        Color32::WHITE,
    );
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
