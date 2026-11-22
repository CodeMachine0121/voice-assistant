//! Friendly names for the shortcut pickers.
use voice_core::Shortcut;

/// Every shortcut the user can pick, in picker order.
pub const ALL_SHORTCUTS: [Shortcut; 5] = [
    Shortcut::RightOption,
    Shortcut::RightCommand,
    Shortcut::Fn,
    Shortcut::ControlOptionSpace,
    Shortcut::ControlShiftSpace,
];

pub fn friendly_name(shortcut: &Shortcut) -> &'static str {
    match shortcut {
        Shortcut::RightOption => "Right Option ⌥ (hold)",
        Shortcut::RightCommand => "Right Command ⌘ (hold)",
        Shortcut::Fn => "Fn (hold)",
        Shortcut::ControlOptionSpace => "Control + Option + Space",
        Shortcut::ControlShiftSpace => "Control + Shift + Space",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_variant_is_offered() {
        // Fails to compile when a variant is added without being listed here.
        let index = |s: &Shortcut| match s {
            Shortcut::RightOption => 0,
            Shortcut::RightCommand => 1,
            Shortcut::Fn => 2,
            Shortcut::ControlOptionSpace => 3,
            Shortcut::ControlShiftSpace => 4,
        };
        let indices: HashSet<_> = ALL_SHORTCUTS.iter().map(index).collect();
        assert_eq!(indices.len(), ALL_SHORTCUTS.len());
    }

    #[test]
    fn names_are_friendly_and_distinct() {
        let names: HashSet<_> = ALL_SHORTCUTS.iter().map(friendly_name).collect();
        assert_eq!(names.len(), ALL_SHORTCUTS.len());
        assert_eq!(
            friendly_name(&Shortcut::RightOption),
            "Right Option ⌥ (hold)"
        );
        assert_eq!(
            friendly_name(&Shortcut::RightCommand),
            "Right Command ⌘ (hold)"
        );
        assert_eq!(friendly_name(&Shortcut::Fn), "Fn (hold)");
        assert_eq!(
            friendly_name(&Shortcut::ControlOptionSpace),
            "Control + Option + Space"
        );
        assert_eq!(
            friendly_name(&Shortcut::ControlShiftSpace),
            "Control + Shift + Space"
        );
    }
}
