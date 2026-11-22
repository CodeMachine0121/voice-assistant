//! Pure keyboard interpretation: raw key events in, shortcut events out.
//!
//! Nothing here talks to the OS, so every mapping rule is unit-tested. The event
//! tap in `shortcuts` only decodes `CGEvent`s into [`KeyInput`] and forwards
//! whatever [`KeyInterpreter::on_key`] returns.
//!
//! Push-to-talk on a modifier-only shortcut counts only while that modifier is
//! held *alone*: if any other key or modifier is pressed meanwhile (for example
//! right-Option+e to type an accent), `PushToTalkUp` is emitted immediately and
//! the later release of the modifier is swallowed. A quick accent therefore ends
//! as a press shorter than the minimum dictation, which the session controller
//! discards silently.
use voice_core::{Shortcut, ShortcutEvent};

/// Virtual key codes (Carbon `kVK_*`) the interpreter cares about.
pub mod keycode {
    pub const SPACE: u16 = 49;
    pub const ESCAPE: u16 = 53;
    pub const RIGHT_COMMAND: u16 = 54;
    pub const RIGHT_OPTION: u16 = 61;
    pub const FUNCTION: u16 = 63;
}

/// `CGEventFlags` bits, both device-independent and device-dependent (`NX_DEVICE*`).
pub mod flag {
    pub const LEFT_CONTROL: u64 = 0x0000_0001;
    pub const LEFT_SHIFT: u64 = 0x0000_0002;
    pub const RIGHT_SHIFT: u64 = 0x0000_0004;
    pub const LEFT_COMMAND: u64 = 0x0000_0008;
    pub const RIGHT_COMMAND: u64 = 0x0000_0010;
    pub const LEFT_OPTION: u64 = 0x0000_0020;
    pub const RIGHT_OPTION: u64 = 0x0000_0040;
    pub const RIGHT_CONTROL: u64 = 0x0000_2000;
    pub const SHIFT: u64 = 0x0002_0000;
    pub const CONTROL: u64 = 0x0004_0000;
    pub const OPTION: u64 = 0x0008_0000;
    pub const COMMAND: u64 = 0x0010_0000;
    pub const FUNCTION: u64 = 0x0080_0000;

    /// One bit per physical modifier key.
    pub const PHYSICAL_MODIFIERS: u64 = LEFT_CONTROL
        | LEFT_SHIFT
        | RIGHT_SHIFT
        | LEFT_COMMAND
        | RIGHT_COMMAND
        | LEFT_OPTION
        | RIGHT_OPTION
        | RIGHT_CONTROL
        | FUNCTION;
}

/// The two shortcuts the listener watches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutConfig {
    pub push_to_talk: Shortcut,
    pub hands_free: Shortcut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Down,
    Up,
    /// A modifier key changed state; `KeyInput::flags` tells the new state.
    FlagsChanged,
}

/// One keyboard event, decoded from the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyInput {
    pub kind: KeyKind,
    pub keycode: u16,
    /// Raw `CGEventFlags` after the event.
    pub flags: u64,
    /// The key is auto-repeating (held down).
    pub autorepeat: bool,
}

/// How a shortcut is physically triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trigger {
    /// A single modifier key held on its own.
    Modifier { keycode: u16, bit: u64 },
    /// Space with exactly these modifiers (Control, Option, Shift, Command).
    Combo { modifiers: u64 },
}

const COMBO_MODIFIERS: u64 = flag::CONTROL | flag::OPTION | flag::SHIFT | flag::COMMAND;

fn trigger(shortcut: &Shortcut) -> Trigger {
    match shortcut {
        Shortcut::RightOption => Trigger::Modifier {
            keycode: keycode::RIGHT_OPTION,
            bit: flag::RIGHT_OPTION,
        },
        Shortcut::RightCommand => Trigger::Modifier {
            keycode: keycode::RIGHT_COMMAND,
            bit: flag::RIGHT_COMMAND,
        },
        Shortcut::Fn => Trigger::Modifier {
            keycode: keycode::FUNCTION,
            bit: flag::FUNCTION,
        },
        Shortcut::ControlOptionSpace => Trigger::Combo {
            modifiers: flag::CONTROL | flag::OPTION,
        },
        Shortcut::ControlShiftSpace => Trigger::Combo {
            modifiers: flag::CONTROL | flag::SHIFT,
        },
    }
}

impl Trigger {
    fn keycode(self) -> u16 {
        match self {
            Trigger::Modifier { keycode, .. } => keycode,
            Trigger::Combo { .. } => keycode::SPACE,
        }
    }

    /// The modifier itself went down while no other modifier is held.
    fn pressed_alone(self, input: &KeyInput) -> bool {
        match self {
            Trigger::Modifier { keycode, bit } => {
                input.kind == KeyKind::FlagsChanged
                    && input.keycode == keycode
                    && input.flags & flag::PHYSICAL_MODIFIERS == bit
            }
            Trigger::Combo { .. } => false,
        }
    }

    fn combo_pressed(self, input: &KeyInput) -> bool {
        match self {
            Trigger::Combo { modifiers } => {
                input.kind == KeyKind::Down
                    && input.keycode == keycode::SPACE
                    && input.flags & COMBO_MODIFIERS == modifiers
            }
            Trigger::Modifier { .. } => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PushToTalk {
    Idle,
    /// Held; `PushToTalkDown` was emitted.
    Held,
    /// Another key interrupted the hold; `PushToTalkUp` was already emitted and
    /// the shortcut's release must be ignored.
    Interrupted,
}

/// Turns raw key events into shortcut events. Deterministic, no I/O.
#[derive(Debug, Clone)]
pub struct KeyInterpreter {
    push_to_talk: Trigger,
    hands_free: Trigger,
    state: PushToTalk,
}

impl KeyInterpreter {
    pub fn new(config: &ShortcutConfig) -> Self {
        Self {
            push_to_talk: trigger(&config.push_to_talk),
            hands_free: trigger(&config.hands_free),
            state: PushToTalk::Idle,
        }
    }

    /// Switches to new shortcuts. A push-to-talk hold in progress ends with `PushToTalkUp`.
    pub fn reconfigure(&mut self, config: &ShortcutConfig) -> Option<ShortcutEvent> {
        let was_held = self.state == PushToTalk::Held;
        *self = Self::new(config);
        was_held.then_some(ShortcutEvent::PushToTalkUp)
    }

    pub fn on_key(&mut self, input: KeyInput) -> Option<ShortcutEvent> {
        if self.state != PushToTalk::Idle {
            return self.while_pushed(input);
        }
        if input.autorepeat {
            return None;
        }
        if input.kind == KeyKind::Down && input.keycode == keycode::ESCAPE {
            return Some(ShortcutEvent::EscapePressed);
        }
        if self.push_to_talk.pressed_alone(&input) || self.push_to_talk.combo_pressed(&input) {
            self.state = PushToTalk::Held;
            return Some(ShortcutEvent::PushToTalkDown);
        }
        if self.hands_free.pressed_alone(&input) || self.hands_free.combo_pressed(&input) {
            return Some(ShortcutEvent::HandsFreePressed);
        }
        None
    }

    /// Push-to-talk is held or was interrupted: watch for its release and for
    /// anything else pressed meanwhile.
    fn while_pushed(&mut self, input: KeyInput) -> Option<ShortcutEvent> {
        let held = self.state == PushToTalk::Held;
        if self.is_release(&input) {
            self.state = PushToTalk::Idle;
            return held.then_some(ShortcutEvent::PushToTalkUp);
        }
        let other_press = input.keycode != self.push_to_talk.keycode()
            && match self.push_to_talk {
                Trigger::Modifier { .. } => input.kind != KeyKind::Up,
                Trigger::Combo { .. } => input.kind == KeyKind::Down,
            };
        if !other_press {
            return None;
        }
        self.state = PushToTalk::Interrupted;
        if input.kind == KeyKind::Down && input.keycode == keycode::ESCAPE {
            Some(ShortcutEvent::EscapePressed)
        } else {
            held.then_some(ShortcutEvent::PushToTalkUp)
        }
    }

    fn is_release(&self, input: &KeyInput) -> bool {
        match self.push_to_talk {
            Trigger::Modifier { keycode, bit } => {
                input.kind == KeyKind::FlagsChanged
                    && input.keycode == keycode
                    && input.flags & bit == 0
            }
            Trigger::Combo { .. } => input.kind == KeyKind::Up && input.keycode == keycode::SPACE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voice_core::ShortcutEvent::*;

    const KEY_E: u16 = 14;
    const LEFT_OPTION: u16 = 58;
    const LEFT_SHIFT: u16 = 56;

    fn config(push_to_talk: Shortcut, hands_free: Shortcut) -> ShortcutConfig {
        ShortcutConfig {
            push_to_talk,
            hands_free,
        }
    }

    fn defaults() -> KeyInterpreter {
        KeyInterpreter::new(&config(Shortcut::RightOption, Shortcut::ControlOptionSpace))
    }

    fn flags_changed(keycode: u16, flags: u64) -> KeyInput {
        KeyInput {
            kind: KeyKind::FlagsChanged,
            keycode,
            flags,
            autorepeat: false,
        }
    }

    fn down(keycode: u16, flags: u64) -> KeyInput {
        KeyInput {
            kind: KeyKind::Down,
            keycode,
            flags,
            autorepeat: false,
        }
    }

    fn repeat(keycode: u16, flags: u64) -> KeyInput {
        KeyInput {
            autorepeat: true,
            ..down(keycode, flags)
        }
    }

    fn up(keycode: u16, flags: u64) -> KeyInput {
        KeyInput {
            kind: KeyKind::Up,
            ..down(keycode, flags)
        }
    }

    const RIGHT_OPTION_HELD: u64 = flag::OPTION | flag::RIGHT_OPTION;
    const CONTROL_OPTION: u64 =
        flag::CONTROL | flag::LEFT_CONTROL | flag::OPTION | flag::LEFT_OPTION;

    fn press_right_option() -> KeyInput {
        flags_changed(keycode::RIGHT_OPTION, RIGHT_OPTION_HELD)
    }

    fn release_right_option() -> KeyInput {
        flags_changed(keycode::RIGHT_OPTION, 0)
    }

    fn feed(interpreter: &mut KeyInterpreter, inputs: &[KeyInput]) -> Vec<Option<ShortcutEvent>> {
        inputs.iter().map(|i| interpreter.on_key(*i)).collect()
    }

    #[test]
    fn holding_right_option_alone_starts_push_to_talk() {
        let mut keys = defaults();

        let event = keys.on_key(press_right_option());

        assert_eq!(event, Some(PushToTalkDown));
    }

    #[test]
    fn releasing_right_option_ends_push_to_talk() {
        let mut keys = defaults();

        let events = feed(&mut keys, &[press_right_option(), release_right_option()]);

        assert_eq!(events, [Some(PushToTalkDown), Some(PushToTalkUp)]);
    }

    #[test]
    fn repeated_press_report_while_held_is_ignored() {
        let mut keys = defaults();

        let events = feed(&mut keys, &[press_right_option(), press_right_option()]);

        assert_eq!(events, [Some(PushToTalkDown), None]);
    }

    #[test]
    fn right_option_with_another_modifier_already_held_is_not_push_to_talk() {
        for other in [
            flag::SHIFT | flag::LEFT_SHIFT,
            flag::CONTROL | flag::LEFT_CONTROL,
            flag::COMMAND | flag::LEFT_COMMAND,
            flag::LEFT_OPTION,
        ] {
            let mut keys = defaults();

            let events = feed(
                &mut keys,
                &[
                    flags_changed(keycode::RIGHT_OPTION, RIGHT_OPTION_HELD | other),
                    flags_changed(keycode::RIGHT_OPTION, other),
                ],
            );

            assert_eq!(events, [None, None], "with other modifier {other:#x}");
        }
    }

    #[test]
    fn typing_an_accent_with_right_option_ends_push_to_talk_at_once() {
        let mut keys = defaults();

        let events = feed(
            &mut keys,
            &[
                press_right_option(),
                down(KEY_E, RIGHT_OPTION_HELD),
                up(KEY_E, RIGHT_OPTION_HELD),
                release_right_option(),
            ],
        );

        assert_eq!(
            events,
            [Some(PushToTalkDown), Some(PushToTalkUp), None, None]
        );
    }

    #[test]
    fn pressing_another_modifier_while_holding_ends_push_to_talk() {
        let mut keys = defaults();

        let events = feed(
            &mut keys,
            &[
                press_right_option(),
                flags_changed(
                    LEFT_SHIFT,
                    RIGHT_OPTION_HELD | flag::SHIFT | flag::LEFT_SHIFT,
                ),
                flags_changed(LEFT_SHIFT, RIGHT_OPTION_HELD),
                release_right_option(),
            ],
        );

        assert_eq!(
            events,
            [Some(PushToTalkDown), Some(PushToTalkUp), None, None]
        );
    }

    #[test]
    fn push_to_talk_works_again_after_an_interrupted_hold() {
        let mut keys = defaults();
        feed(
            &mut keys,
            &[
                press_right_option(),
                down(KEY_E, RIGHT_OPTION_HELD),
                release_right_option(),
            ],
        );

        let event = keys.on_key(press_right_option());

        assert_eq!(event, Some(PushToTalkDown));
    }

    #[test]
    fn right_command_can_be_the_push_to_talk_shortcut() {
        let mut keys = KeyInterpreter::new(&config(
            Shortcut::RightCommand,
            Shortcut::ControlOptionSpace,
        ));

        let events = feed(
            &mut keys,
            &[
                flags_changed(keycode::RIGHT_COMMAND, flag::COMMAND | flag::RIGHT_COMMAND),
                flags_changed(keycode::RIGHT_COMMAND, 0),
            ],
        );

        assert_eq!(events, [Some(PushToTalkDown), Some(PushToTalkUp)]);
    }

    #[test]
    fn fn_can_be_the_push_to_talk_shortcut() {
        let mut keys = KeyInterpreter::new(&config(Shortcut::Fn, Shortcut::ControlOptionSpace));

        let events = feed(
            &mut keys,
            &[
                flags_changed(keycode::FUNCTION, flag::FUNCTION),
                flags_changed(keycode::FUNCTION, 0),
            ],
        );

        assert_eq!(events, [Some(PushToTalkDown), Some(PushToTalkUp)]);
    }

    #[test]
    fn left_option_is_not_the_right_option_shortcut() {
        let mut keys = defaults();

        let events = feed(
            &mut keys,
            &[
                flags_changed(LEFT_OPTION, flag::OPTION | flag::LEFT_OPTION),
                flags_changed(LEFT_OPTION, 0),
            ],
        );

        assert_eq!(events, [None, None]);
    }

    #[test]
    fn control_option_space_toggles_hands_free_on_every_press() {
        let mut keys = defaults();

        let events = feed(
            &mut keys,
            &[
                down(keycode::SPACE, CONTROL_OPTION),
                up(keycode::SPACE, CONTROL_OPTION),
                down(keycode::SPACE, CONTROL_OPTION),
            ],
        );

        assert_eq!(
            events,
            [Some(HandsFreePressed), None, Some(HandsFreePressed)]
        );
    }

    #[test]
    fn auto_repeat_of_the_hands_free_shortcut_is_ignored() {
        let mut keys = defaults();

        let events = feed(
            &mut keys,
            &[
                down(keycode::SPACE, CONTROL_OPTION),
                repeat(keycode::SPACE, CONTROL_OPTION),
            ],
        );

        assert_eq!(events, [Some(HandsFreePressed), None]);
    }

    #[test]
    fn space_with_other_modifiers_is_not_hands_free() {
        for flags in [
            CONTROL_OPTION | flag::COMMAND,
            flag::CONTROL,
            flag::OPTION,
            0,
        ] {
            let mut keys = defaults();

            let event = keys.on_key(down(keycode::SPACE, flags));

            assert_eq!(event, None, "with flags {flags:#x}");
        }
    }

    #[test]
    fn control_shift_space_can_be_the_hands_free_shortcut() {
        let mut keys =
            KeyInterpreter::new(&config(Shortcut::RightOption, Shortcut::ControlShiftSpace));

        let events = feed(
            &mut keys,
            &[
                down(keycode::SPACE, flag::CONTROL | flag::SHIFT),
                down(keycode::SPACE, CONTROL_OPTION),
            ],
        );

        assert_eq!(events, [Some(HandsFreePressed), None]);
    }

    #[test]
    fn escape_while_holding_push_to_talk_cancels_and_release_is_silent() {
        let mut keys = defaults();

        let events = feed(
            &mut keys,
            &[
                press_right_option(),
                down(keycode::ESCAPE, RIGHT_OPTION_HELD),
                release_right_option(),
            ],
        );

        assert_eq!(events, [Some(PushToTalkDown), Some(EscapePressed), None]);
    }

    #[test]
    fn escape_when_not_holding_reports_escape() {
        let mut keys = defaults();

        let event = keys.on_key(down(keycode::ESCAPE, 0));

        assert_eq!(event, Some(EscapePressed));
    }

    #[test]
    fn auto_repeat_of_escape_is_ignored() {
        let mut keys = defaults();

        let event = keys.on_key(repeat(keycode::ESCAPE, 0));

        assert_eq!(event, None);
    }

    #[test]
    fn unrelated_keys_produce_nothing() {
        let mut keys = defaults();

        let events = feed(
            &mut keys,
            &[
                down(KEY_E, 0),
                up(KEY_E, 0),
                up(keycode::ESCAPE, 0),
                flags_changed(LEFT_SHIFT, flag::SHIFT | flag::LEFT_SHIFT),
                flags_changed(LEFT_SHIFT, 0),
                release_right_option(),
            ],
        );

        assert_eq!(events, [None; 6]);
    }

    #[test]
    fn reconfigured_shortcut_takes_effect_immediately() {
        let mut keys = defaults();
        let produced = keys.reconfigure(&config(
            Shortcut::RightCommand,
            Shortcut::ControlOptionSpace,
        ));

        let events = feed(
            &mut keys,
            &[
                press_right_option(),
                release_right_option(),
                flags_changed(keycode::RIGHT_COMMAND, flag::COMMAND | flag::RIGHT_COMMAND),
            ],
        );

        assert_eq!(
            (produced, events),
            (None, vec![None, None, Some(PushToTalkDown)])
        );
    }

    #[test]
    fn reconfiguring_while_holding_ends_push_to_talk() {
        let mut keys = defaults();
        keys.on_key(press_right_option());

        let produced = keys.reconfigure(&config(
            Shortcut::RightCommand,
            Shortcut::ControlOptionSpace,
        ));

        assert_eq!(produced, Some(PushToTalkUp));
    }

    #[test]
    fn modifier_only_hands_free_shortcut_toggles_on_press() {
        let mut keys = KeyInterpreter::new(&config(Shortcut::RightOption, Shortcut::RightCommand));

        let events = feed(
            &mut keys,
            &[
                flags_changed(keycode::RIGHT_COMMAND, flag::COMMAND | flag::RIGHT_COMMAND),
                flags_changed(keycode::RIGHT_COMMAND, 0),
            ],
        );

        assert_eq!(events, [Some(HandsFreePressed), None]);
    }

    #[test]
    fn combo_push_to_talk_is_held_while_space_is_down() {
        let mut keys = KeyInterpreter::new(&config(
            Shortcut::ControlOptionSpace,
            Shortcut::ControlShiftSpace,
        ));

        let events = feed(
            &mut keys,
            &[
                down(keycode::SPACE, CONTROL_OPTION),
                repeat(keycode::SPACE, CONTROL_OPTION),
                flags_changed(LEFT_OPTION, flag::CONTROL | flag::LEFT_CONTROL),
                up(keycode::SPACE, flag::CONTROL | flag::LEFT_CONTROL),
            ],
        );

        assert_eq!(
            events,
            [Some(PushToTalkDown), None, None, Some(PushToTalkUp)]
        );
    }

    #[test]
    fn another_key_while_holding_combo_push_to_talk_ends_it() {
        let mut keys = KeyInterpreter::new(&config(
            Shortcut::ControlOptionSpace,
            Shortcut::ControlShiftSpace,
        ));

        let events = feed(
            &mut keys,
            &[
                down(keycode::SPACE, CONTROL_OPTION),
                down(KEY_E, CONTROL_OPTION),
                up(keycode::SPACE, CONTROL_OPTION),
            ],
        );

        assert_eq!(events, [Some(PushToTalkDown), Some(PushToTalkUp), None]);
    }
}
