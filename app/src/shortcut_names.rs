//! Friendly names and key caps for the shortcut pickers.
use voice_core::Shortcut;

/// Every shortcut the user can pick, in picker order.
pub const ALL_SHORTCUTS: [Shortcut; 7] = Shortcut::ALL;

pub fn friendly_name(shortcut: &Shortcut) -> &'static str {
    match shortcut {
        Shortcut::TapCommand => "Tap Command",
        Shortcut::TapCommandShift => "Tap Command + Shift",
        Shortcut::OptionSpace => "Option + Space",
        Shortcut::TapRightOption => "Tap right Option",
        Shortcut::TapRightCommand => "Tap right Command",
        Shortcut::TapFn => "Tap Fn",
        Shortcut::ControlShiftSpace => "Control + Shift + Space",
    }
}

/// The keys of a shortcut, one entry per key cap.
pub fn key_caps(shortcut: &Shortcut) -> &'static [&'static str] {
    match shortcut {
        Shortcut::TapCommand => &["⌘"],
        Shortcut::TapCommandShift => &["⌘", "⇧"],
        Shortcut::OptionSpace => &["⌥", "Space"],
        Shortcut::TapRightOption => &["Right ⌥"],
        Shortcut::TapRightCommand => &["Right ⌘"],
        Shortcut::TapFn => &["fn"],
        Shortcut::ControlShiftSpace => &["⌃", "⇧", "Space"],
    }
}

/// Key caps as one line of text, e.g. `⌘ ⇧`.
pub fn caps_text(shortcut: &Shortcut) -> String {
    key_caps(shortcut).join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_variant_is_offered() {
        // Fails to compile when a variant is added without being listed here.
        let index = |s: &Shortcut| match s {
            Shortcut::TapCommand => 0,
            Shortcut::TapCommandShift => 1,
            Shortcut::OptionSpace => 2,
            Shortcut::TapRightOption => 3,
            Shortcut::TapRightCommand => 4,
            Shortcut::TapFn => 5,
            Shortcut::ControlShiftSpace => 6,
        };
        let indices: HashSet<_> = ALL_SHORTCUTS.iter().map(index).collect();
        assert_eq!(indices.len(), ALL_SHORTCUTS.len());
    }

    #[test]
    fn names_are_friendly_and_distinct() {
        let names: Vec<_> = ALL_SHORTCUTS.iter().map(friendly_name).collect();
        assert_eq!(
            names,
            [
                "Tap Command",
                "Tap Command + Shift",
                "Option + Space",
                "Tap right Option",
                "Tap right Command",
                "Tap Fn",
                "Control + Shift + Space",
            ]
        );
        let distinct: HashSet<_> = names.iter().collect();
        assert_eq!(distinct.len(), names.len());
    }

    #[test]
    fn key_caps_read_like_the_keyboard() {
        let caps: Vec<_> = ALL_SHORTCUTS.iter().map(caps_text).collect();
        assert_eq!(
            caps,
            [
                "⌘",
                "⌘ ⇧",
                "⌥ Space",
                "Right ⌥",
                "Right ⌘",
                "fn",
                "⌃ ⇧ Space"
            ]
        );
        assert_eq!(key_caps(&Shortcut::ControlShiftSpace), ["⌃", "⇧", "Space"]);
    }
}
