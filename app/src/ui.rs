//! Drawing. Thin by design: everything that can be decided without a window lives
//! in the view-model modules; this layer only turns it into egui calls.
pub mod history;
pub mod indicator;
pub mod settings;

use eframe::egui::Color32;
use voice_core::Action;

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
