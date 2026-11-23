//! Drawing. Thin by design: everything that can be decided without a window lives
//! in the view-model modules; this layer only turns it into egui calls.
pub mod history;
pub mod indicator;
pub mod settings;

use crate::focus::FocusRequest;
use eframe::egui::{self, Color32, ViewportCommand};
use std::time::Instant;
use voice_core::Action;

/// Inside a window's viewport: asks for keyboard focus while `focus` is pending.
pub fn keep_focusing(ctx: &egui::Context, focus: &mut FocusRequest, now: Instant) {
    let focused = ctx.input(|i| i.viewport().focused == Some(true));
    if focus.poll(focused, now) {
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        // Look again next frame: a new window is only shown after its first frame.
        ctx.request_repaint();
    }
}

/// Chip colour per action: Dictate blue, English purple, Edit amber.
pub fn action_colour(action: Action) -> Color32 {
    match action {
        Action::Dictate => Color32::from_rgb(10, 132, 255),
        Action::DictateInEnglish => Color32::from_rgb(175, 82, 222),
        Action::EditSelection => Color32::from_rgb(255, 159, 10),
    }
}

/// Colour for problems and conflicts.
pub const PROBLEM_COLOUR: Color32 = Color32::from_rgb(255, 99, 88);
/// Colour for saved / granted.
pub const OK_COLOUR: Color32 = Color32::from_rgb(52, 199, 89);

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{RawInput, ViewportId, ViewportInfo};

    /// Runs one frame of a window that has (or lacks) keyboard focus and returns the
    /// commands it sent to its own viewport.
    fn frame(focused: bool, focus: &mut FocusRequest, now: Instant) -> Vec<ViewportCommand> {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.viewports.insert(
            ViewportId::ROOT,
            ViewportInfo {
                focused: Some(focused),
                ..Default::default()
            },
        );
        let mut output = ctx.run_ui(input, |ui| keep_focusing(ui.ctx(), focus, now));
        output.textures_delta.clear();
        output
            .viewport_output
            .remove(&ViewportId::ROOT)
            .map(|v| v.commands)
            .unwrap_or_default()
    }

    #[test]
    fn a_pending_request_asks_an_unfocused_window_for_focus() {
        let now = Instant::now();
        let mut focus = FocusRequest::default();
        focus.request(now);
        assert!(frame(false, &mut focus, now).contains(&ViewportCommand::Focus));
    }

    #[test]
    fn a_focused_window_is_left_alone() {
        let now = Instant::now();
        let mut focus = FocusRequest::default();
        focus.request(now);
        assert!(!frame(true, &mut focus, now).contains(&ViewportCommand::Focus));
        assert!(!frame(false, &mut focus, now).contains(&ViewportCommand::Focus));
    }

    #[test]
    fn without_a_request_focus_is_never_taken() {
        let mut focus = FocusRequest::default();
        assert!(!frame(false, &mut focus, Instant::now()).contains(&ViewportCommand::Focus));
    }
}
