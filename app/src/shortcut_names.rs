//! Key caps of the shortcuts, as Settings shows them.
use voice_core::Shortcut;

/// The keys of a shortcut, one entry per key cap. Every modifier is the right-hand
/// key, and the caps say so.
pub fn key_caps(shortcut: Shortcut) -> &'static [&'static str] {
    match shortcut {
        Shortcut::TapRightCommand => &["Right ⌘"],
        Shortcut::TapRightCommandRightShift => &["Right ⌘", "Right ⇧"],
        Shortcut::RightOptionSpace => &["Right ⌥", "Space"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voice_core::Action;

    #[test]
    fn each_action_shows_its_right_hand_keys() {
        let caps: Vec<_> = Action::ALL
            .into_iter()
            .map(|action| key_caps(Shortcut::for_action(action)).join(" + "))
            .collect();
        assert_eq!(caps, ["Right ⌘", "Right ⌘ + Right ⇧", "Right ⌥ + Space"]);
    }
}
