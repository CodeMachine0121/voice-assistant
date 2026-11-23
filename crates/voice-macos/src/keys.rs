//! Pure keyboard interpretation: raw key and pointer events in, shortcut events out.
//!
//! Nothing here talks to the OS, so every mapping rule is unit-tested. The event
//! tap in `shortcuts` only decodes `CGEvent`s into [`KeyInput`] and acts on
//! whatever [`KeyInterpreter::on_key`] decides.
//!
//! Two kinds of shortcuts exist:
//!
//! * **Taps** (a modifier, or Command + Shift, pressed and released on its own).
//!   The interpreter remembers every physical modifier pressed since the moment
//!   all modifiers were last up (the *chord*) and whether anything else happened
//!   meanwhile — a key press or a mouse click. When the last modifier goes up, a
//!   clean chord that matches exactly one configured tap fires that action.
//!   Command + C or Command-clicking therefore never starts anything.
//! * **Combos** (Space with exact modifiers). The Space press fires at once and
//!   is swallowed together with its repeats and its release, so no space is typed.
use voice_core::{Action, ActionShortcuts, Settings, Shortcut, ShortcutEvent};

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

/// The shortcut of every action, as the listener watches them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShortcutConfig {
    pub shortcuts: ActionShortcuts,
}

impl From<&Settings> for ShortcutConfig {
    fn from(settings: &Settings) -> Self {
        Self {
            shortcuts: settings.shortcuts.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Down,
    Up,
    /// A modifier key changed state; `KeyInput::flags` tells the new state.
    FlagsChanged,
    /// A mouse button went down.
    Pointer,
}

/// One keyboard or pointer event, decoded from the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyInput {
    pub kind: KeyKind,
    pub keycode: u16,
    /// Raw `CGEventFlags` after the event.
    pub flags: u64,
    /// The key is auto-repeating (held down).
    pub autorepeat: bool,
}

/// What one input means, and whether the front app must not see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyDecision {
    pub event: Option<ShortcutEvent>,
    /// Drop the input instead of passing it on (the Space of a combo shortcut).
    pub consume: bool,
}

impl KeyDecision {
    const PASS: Self = Self {
        event: None,
        consume: false,
    };
    const SWALLOW: Self = Self {
        event: None,
        consume: true,
    };
}

/// How a shortcut is physically triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trigger {
    /// Modifiers pressed and released on their own; the chord must be one of these exactly.
    Tap(&'static [u64]),
    /// Space with exactly these modifiers (Control, Option, Shift, Command).
    Combo(u64),
}

const COMBO_MODIFIERS: u64 = flag::CONTROL | flag::OPTION | flag::SHIFT | flag::COMMAND;

const ONE_COMMAND: &[u64] = &[flag::LEFT_COMMAND, flag::RIGHT_COMMAND];
const ONE_COMMAND_ONE_SHIFT: &[u64] = &[
    flag::LEFT_COMMAND | flag::LEFT_SHIFT,
    flag::LEFT_COMMAND | flag::RIGHT_SHIFT,
    flag::RIGHT_COMMAND | flag::LEFT_SHIFT,
    flag::RIGHT_COMMAND | flag::RIGHT_SHIFT,
];

fn trigger(shortcut: Shortcut) -> Trigger {
    match shortcut {
        Shortcut::TapCommand => Trigger::Tap(ONE_COMMAND),
        Shortcut::TapCommandShift => Trigger::Tap(ONE_COMMAND_ONE_SHIFT),
        Shortcut::TapRightOption => Trigger::Tap(&[flag::RIGHT_OPTION]),
        Shortcut::TapRightCommand => Trigger::Tap(&[flag::RIGHT_COMMAND]),
        Shortcut::TapFn => Trigger::Tap(&[flag::FUNCTION]),
        Shortcut::OptionSpace => Trigger::Combo(flag::OPTION),
        Shortcut::ControlShiftSpace => Trigger::Combo(flag::CONTROL | flag::SHIFT),
    }
}

impl Trigger {
    fn tapped(self, chord: u64) -> bool {
        matches!(self, Trigger::Tap(chords) if chords.contains(&chord))
    }

    fn combo_pressed(self, input: &KeyInput) -> bool {
        matches!(self, Trigger::Combo(modifiers) if input.flags & COMBO_MODIFIERS == modifiers)
    }
}

/// Turns raw key events into shortcut events. Deterministic, no I/O.
#[derive(Debug, Clone)]
pub struct KeyInterpreter {
    triggers: [(Action, Trigger); 3],
    /// Every physical modifier pressed since all modifiers were last up.
    chord: u64,
    /// A key or mouse button went down while a modifier was held.
    interrupted: bool,
    /// The Space of a combo is down: its repeats and its release are swallowed.
    swallowing_space: bool,
}

impl KeyInterpreter {
    pub fn new(config: &ShortcutConfig) -> Self {
        Self {
            triggers: triggers(&config.shortcuts),
            chord: 0,
            interrupted: false,
            swallowing_space: false,
        }
    }

    /// Switches to new shortcuts. Keys already down keep their meaning: a chord in
    /// progress is judged by the new shortcuts on release, a swallowed Space stays swallowed.
    pub fn reconfigure(&mut self, config: &ShortcutConfig) {
        self.triggers = triggers(&config.shortcuts);
    }

    pub fn on_key(&mut self, input: KeyInput) -> KeyDecision {
        match input.kind {
            KeyKind::FlagsChanged => KeyDecision {
                event: self.modifiers_changed(input.flags),
                consume: false,
            },
            KeyKind::Pointer => {
                self.interrupt();
                KeyDecision::PASS
            }
            KeyKind::Down => self.key_down(input),
            KeyKind::Up => self.key_up(input),
        }
    }

    /// Something other than a modifier happened; a modifier held now is not a tap.
    fn interrupt(&mut self) {
        if self.chord != 0 {
            self.interrupted = true;
        }
    }

    fn modifiers_changed(&mut self, flags: u64) -> Option<ShortcutEvent> {
        let held = flags & flag::PHYSICAL_MODIFIERS;
        self.chord |= held;
        if held != 0 {
            return None;
        }
        let (chord, interrupted) = (self.chord, self.interrupted);
        self.chord = 0;
        self.interrupted = false;
        if chord == 0 || interrupted {
            return None;
        }
        let mut tapped = self
            .triggers
            .iter()
            .filter(|(_, trigger)| trigger.tapped(chord));
        match (tapped.next(), tapped.next()) {
            (Some((action, _)), None) => Some(ShortcutEvent::ActionTapped(*action)),
            _ => None,
        }
    }

    fn key_down(&mut self, input: KeyInput) -> KeyDecision {
        self.interrupt();
        if input.keycode == keycode::SPACE {
            if self.swallowing_space && input.autorepeat {
                return KeyDecision::SWALLOW;
            }
            if !input.autorepeat {
                if let Some((action, _)) = self
                    .triggers
                    .iter()
                    .find(|(_, trigger)| trigger.combo_pressed(&input))
                {
                    self.swallowing_space = true;
                    return KeyDecision {
                        event: Some(ShortcutEvent::ActionTapped(*action)),
                        consume: true,
                    };
                }
            }
            self.swallowing_space = false;
            return KeyDecision::PASS;
        }
        if input.keycode == keycode::ESCAPE && !input.autorepeat {
            return KeyDecision {
                event: Some(ShortcutEvent::EscapePressed),
                consume: false,
            };
        }
        KeyDecision::PASS
    }

    fn key_up(&mut self, input: KeyInput) -> KeyDecision {
        if input.keycode == keycode::SPACE && self.swallowing_space {
            self.swallowing_space = false;
            return KeyDecision::SWALLOW;
        }
        KeyDecision::PASS
    }
}

fn triggers(shortcuts: &ActionShortcuts) -> [(Action, Trigger); 3] {
    Action::ALL.map(|action| (action, trigger(*shortcuts.for_action(action))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use voice_core::Action::*;
    use voice_core::ShortcutEvent::*;

    const KEY_C: u16 = 8;
    const KEY_E: u16 = 14;
    const LEFT_COMMAND: u16 = 55;
    const LEFT_SHIFT: u16 = 56;
    const LEFT_OPTION: u16 = 58;
    const LEFT_CONTROL: u16 = 59;
    const RIGHT_SHIFT: u16 = 60;

    const LEFT_COMMAND_HELD: u64 = flag::COMMAND | flag::LEFT_COMMAND;
    const RIGHT_COMMAND_HELD: u64 = flag::COMMAND | flag::RIGHT_COMMAND;
    const LEFT_SHIFT_HELD: u64 = flag::SHIFT | flag::LEFT_SHIFT;
    const LEFT_OPTION_HELD: u64 = flag::OPTION | flag::LEFT_OPTION;
    const RIGHT_OPTION_HELD: u64 = flag::OPTION | flag::RIGHT_OPTION;
    const LEFT_CONTROL_HELD: u64 = flag::CONTROL | flag::LEFT_CONTROL;

    fn defaults() -> KeyInterpreter {
        KeyInterpreter::new(&ShortcutConfig::default())
    }

    fn with(dictate: Shortcut, english: Shortcut, edit_selection: Shortcut) -> KeyInterpreter {
        KeyInterpreter::new(&ShortcutConfig {
            shortcuts: ActionShortcuts {
                dictate,
                english,
                edit_selection,
            },
        })
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

    fn click(flags: u64) -> KeyInput {
        KeyInput {
            kind: KeyKind::Pointer,
            keycode: 0,
            flags,
            autorepeat: false,
        }
    }

    fn events(keys: &mut KeyInterpreter, inputs: &[KeyInput]) -> Vec<ShortcutEvent> {
        inputs
            .iter()
            .filter_map(|input| keys.on_key(*input).event)
            .collect()
    }

    fn decisions(keys: &mut KeyInterpreter, inputs: &[KeyInput]) -> Vec<KeyDecision> {
        inputs.iter().map(|input| keys.on_key(*input)).collect()
    }

    fn fired(action: Action) -> KeyDecision {
        KeyDecision {
            event: Some(ActionTapped(action)),
            consume: false,
        }
    }

    fn command_tap() -> [KeyInput; 2] {
        [
            flags_changed(LEFT_COMMAND, LEFT_COMMAND_HELD),
            flags_changed(LEFT_COMMAND, 0),
        ]
    }

    #[test]
    fn config_takes_every_action_shortcut_from_the_settings() {
        let shortcuts = ActionShortcuts {
            dictate: Shortcut::TapFn,
            english: Shortcut::TapRightOption,
            edit_selection: Shortcut::ControlShiftSpace,
        };
        let settings = Settings {
            shortcuts: shortcuts.clone(),
            ..Settings::default()
        };

        let config = ShortcutConfig::from(&settings);

        assert_eq!(config, ShortcutConfig { shortcuts });
    }

    #[test]
    fn tapping_command_dictates_on_release() {
        let mut keys = defaults();

        let decided = decisions(&mut keys, &command_tap());

        assert_eq!(decided, [KeyDecision::PASS, fired(Dictate)]);
    }

    #[test]
    fn tapping_right_command_also_dictates() {
        let mut keys = defaults();

        let produced = events(
            &mut keys,
            &[
                flags_changed(keycode::RIGHT_COMMAND, RIGHT_COMMAND_HELD),
                flags_changed(keycode::RIGHT_COMMAND, 0),
            ],
        );

        assert_eq!(produced, [ActionTapped(Dictate)]);
    }

    #[test]
    fn command_c_does_not_dictate() {
        let mut keys = defaults();

        let produced = events(
            &mut keys,
            &[
                flags_changed(LEFT_COMMAND, LEFT_COMMAND_HELD),
                down(KEY_C, LEFT_COMMAND_HELD),
                up(KEY_C, LEFT_COMMAND_HELD),
                flags_changed(LEFT_COMMAND, 0),
            ],
        );

        assert_eq!(produced, []);
    }

    #[test]
    fn command_click_does_not_dictate() {
        let mut keys = defaults();

        let produced = events(
            &mut keys,
            &[
                flags_changed(LEFT_COMMAND, LEFT_COMMAND_HELD),
                click(LEFT_COMMAND_HELD),
                flags_changed(LEFT_COMMAND, 0),
            ],
        );

        assert_eq!(produced, []);
    }

    #[test]
    fn a_click_without_modifiers_does_not_spoil_the_next_tap() {
        let mut keys = defaults();
        keys.on_key(click(0));
        keys.on_key(down(KEY_E, 0));

        let produced = events(&mut keys, &command_tap());

        assert_eq!(produced, [ActionTapped(Dictate)]);
    }

    #[test]
    fn command_tap_works_again_after_a_command_shortcut() {
        let mut keys = defaults();
        events(
            &mut keys,
            &[
                flags_changed(LEFT_COMMAND, LEFT_COMMAND_HELD),
                down(KEY_C, LEFT_COMMAND_HELD),
                flags_changed(LEFT_COMMAND, 0),
            ],
        );

        let produced = events(&mut keys, &command_tap());

        assert_eq!(produced, [ActionTapped(Dictate)]);
    }

    #[test]
    fn command_then_shift_dictates_in_english_whichever_is_released_first() {
        let both = LEFT_COMMAND_HELD | LEFT_SHIFT_HELD;
        for releases in [
            [
                flags_changed(LEFT_COMMAND, LEFT_SHIFT_HELD),
                flags_changed(LEFT_SHIFT, 0),
            ],
            [
                flags_changed(LEFT_SHIFT, LEFT_COMMAND_HELD),
                flags_changed(LEFT_COMMAND, 0),
            ],
        ] {
            let mut keys = defaults();
            let mut inputs = vec![
                flags_changed(LEFT_COMMAND, LEFT_COMMAND_HELD),
                flags_changed(LEFT_SHIFT, both),
            ];
            inputs.extend(releases);

            let produced = events(&mut keys, &inputs);

            assert_eq!(produced, [ActionTapped(DictateInEnglish)]);
        }
    }

    #[test]
    fn shift_then_command_dictates_in_english() {
        let mut keys = defaults();

        let produced = events(
            &mut keys,
            &[
                flags_changed(RIGHT_SHIFT, flag::SHIFT | flag::RIGHT_SHIFT),
                flags_changed(
                    keycode::RIGHT_COMMAND,
                    flag::SHIFT | flag::RIGHT_SHIFT | RIGHT_COMMAND_HELD,
                ),
                flags_changed(keycode::RIGHT_COMMAND, flag::SHIFT | flag::RIGHT_SHIFT),
                flags_changed(RIGHT_SHIFT, 0),
            ],
        );

        assert_eq!(produced, [ActionTapped(DictateInEnglish)]);
    }

    #[test]
    fn command_shift_with_a_letter_does_nothing() {
        let both = LEFT_COMMAND_HELD | LEFT_SHIFT_HELD;
        let mut keys = defaults();

        let produced = events(
            &mut keys,
            &[
                flags_changed(LEFT_COMMAND, LEFT_COMMAND_HELD),
                flags_changed(LEFT_SHIFT, both),
                down(KEY_E, both),
                flags_changed(LEFT_SHIFT, LEFT_COMMAND_HELD),
                flags_changed(LEFT_COMMAND, 0),
            ],
        );

        assert_eq!(produced, []);
    }

    #[test]
    fn chords_that_match_no_shortcut_do_nothing() {
        for (key, held) in [
            (LEFT_SHIFT, LEFT_SHIFT_HELD),
            (LEFT_OPTION, LEFT_OPTION_HELD),
            (keycode::RIGHT_OPTION, RIGHT_OPTION_HELD),
            (LEFT_CONTROL, LEFT_CONTROL_HELD),
            (keycode::FUNCTION, flag::FUNCTION),
            (LEFT_COMMAND, LEFT_COMMAND_HELD | flag::RIGHT_COMMAND),
            (LEFT_COMMAND, LEFT_COMMAND_HELD | LEFT_OPTION_HELD),
            (
                LEFT_COMMAND,
                LEFT_COMMAND_HELD | LEFT_SHIFT_HELD | flag::RIGHT_SHIFT,
            ),
        ] {
            let mut keys = defaults();

            let produced = events(
                &mut keys,
                &[flags_changed(key, held), flags_changed(key, 0)],
            );

            assert_eq!(produced, [], "chord {held:#x}");
        }
    }

    #[test]
    fn option_space_edits_the_selection_and_the_space_is_not_typed() {
        let mut keys = defaults();

        let decided = decisions(
            &mut keys,
            &[
                flags_changed(LEFT_OPTION, LEFT_OPTION_HELD),
                down(keycode::SPACE, LEFT_OPTION_HELD),
                repeat(keycode::SPACE, LEFT_OPTION_HELD),
                up(keycode::SPACE, LEFT_OPTION_HELD),
                flags_changed(LEFT_OPTION, 0),
            ],
        );

        assert_eq!(
            decided,
            [
                KeyDecision::PASS,
                KeyDecision {
                    event: Some(ActionTapped(EditSelection)),
                    consume: true,
                },
                KeyDecision::SWALLOW,
                KeyDecision::SWALLOW,
                KeyDecision::PASS,
            ]
        );
    }

    #[test]
    fn right_option_space_also_edits_the_selection() {
        let mut keys = defaults();

        let decided = keys.on_key(down(keycode::SPACE, RIGHT_OPTION_HELD));

        assert_eq!(
            decided,
            KeyDecision {
                event: Some(ActionTapped(EditSelection)),
                consume: true,
            }
        );
    }

    #[test]
    fn space_released_after_option_is_still_swallowed() {
        let mut keys = defaults();

        let decided = decisions(
            &mut keys,
            &[
                flags_changed(LEFT_OPTION, LEFT_OPTION_HELD),
                down(keycode::SPACE, LEFT_OPTION_HELD),
                flags_changed(LEFT_OPTION, 0),
                up(keycode::SPACE, 0),
            ],
        );

        assert_eq!(
            decided.iter().map(|d| d.consume).collect::<Vec<_>>(),
            [false, true, false, true]
        );
    }

    #[test]
    fn plain_space_and_space_with_other_modifiers_pass_through() {
        for flags in [
            0,
            LEFT_SHIFT_HELD,
            LEFT_OPTION_HELD | LEFT_SHIFT_HELD,
            LEFT_OPTION_HELD | LEFT_COMMAND_HELD,
            LEFT_CONTROL_HELD,
        ] {
            let mut keys = defaults();

            let decided = decisions(
                &mut keys,
                &[down(keycode::SPACE, flags), up(keycode::SPACE, flags)],
            );

            assert_eq!(decided, [KeyDecision::PASS; 2], "flags {flags:#x}");
        }
    }

    #[test]
    fn plain_space_after_a_combo_is_typed() {
        let mut keys = defaults();
        decisions(
            &mut keys,
            &[
                down(keycode::SPACE, LEFT_OPTION_HELD),
                up(keycode::SPACE, LEFT_OPTION_HELD),
            ],
        );

        let decided = decisions(
            &mut keys,
            &[
                down(keycode::SPACE, 0),
                repeat(keycode::SPACE, 0),
                up(keycode::SPACE, 0),
            ],
        );

        assert_eq!(decided, [KeyDecision::PASS; 3]);
    }

    #[test]
    fn option_alone_does_nothing() {
        let mut keys = defaults();

        let decided = decisions(
            &mut keys,
            &[
                flags_changed(LEFT_OPTION, LEFT_OPTION_HELD),
                flags_changed(LEFT_OPTION, 0),
            ],
        );

        assert_eq!(decided, [KeyDecision::PASS; 2]);
    }

    #[test]
    fn releasing_option_after_a_combo_is_not_a_tap() {
        let mut keys = with(
            Shortcut::TapCommand,
            Shortcut::TapRightOption,
            Shortcut::OptionSpace,
        );

        let produced = events(
            &mut keys,
            &[
                flags_changed(keycode::RIGHT_OPTION, RIGHT_OPTION_HELD),
                down(keycode::SPACE, RIGHT_OPTION_HELD),
                up(keycode::SPACE, RIGHT_OPTION_HELD),
                flags_changed(keycode::RIGHT_OPTION, 0),
            ],
        );

        assert_eq!(produced, [ActionTapped(EditSelection)]);
    }

    #[test]
    fn auto_repeat_of_a_combo_space_does_not_fire_again() {
        let mut keys = defaults();

        let produced = events(
            &mut keys,
            &[
                down(keycode::SPACE, LEFT_OPTION_HELD),
                repeat(keycode::SPACE, LEFT_OPTION_HELD),
                repeat(keycode::SPACE, LEFT_OPTION_HELD),
            ],
        );

        assert_eq!(produced, [ActionTapped(EditSelection)]);
    }

    #[test]
    fn escape_cancels_and_is_passed_on() {
        let mut keys = defaults();

        let decided = keys.on_key(down(keycode::ESCAPE, 0));

        assert_eq!(
            decided,
            KeyDecision {
                event: Some(EscapePressed),
                consume: false,
            }
        );
    }

    #[test]
    fn escape_while_holding_command_cancels_and_is_not_a_tap() {
        let mut keys = defaults();

        let produced = events(
            &mut keys,
            &[
                flags_changed(LEFT_COMMAND, LEFT_COMMAND_HELD),
                down(keycode::ESCAPE, LEFT_COMMAND_HELD),
                flags_changed(LEFT_COMMAND, 0),
            ],
        );

        assert_eq!(produced, [EscapePressed]);
    }

    #[test]
    fn auto_repeat_and_release_of_escape_are_ignored() {
        let mut keys = defaults();

        let decided = decisions(
            &mut keys,
            &[repeat(keycode::ESCAPE, 0), up(keycode::ESCAPE, 0)],
        );

        assert_eq!(decided, [KeyDecision::PASS; 2]);
    }

    #[test]
    fn unrelated_keys_produce_nothing() {
        let mut keys = defaults();

        let decided = decisions(
            &mut keys,
            &[down(KEY_E, 0), repeat(KEY_E, 0), up(KEY_E, 0), click(0)],
        );

        assert_eq!(decided, [KeyDecision::PASS; 4]);
    }

    #[test]
    fn caps_lock_and_other_flag_changes_without_modifiers_do_nothing() {
        let mut keys = defaults();

        let produced = events(&mut keys, &[flags_changed(57, 0x0001_0000)]);

        assert_eq!(produced, []);
    }

    #[test]
    fn every_tap_shortcut_can_be_chosen() {
        for (shortcut, key, held) in [
            (
                Shortcut::TapRightOption,
                keycode::RIGHT_OPTION,
                RIGHT_OPTION_HELD,
            ),
            (
                Shortcut::TapRightCommand,
                keycode::RIGHT_COMMAND,
                RIGHT_COMMAND_HELD,
            ),
            (Shortcut::TapFn, keycode::FUNCTION, flag::FUNCTION),
        ] {
            let mut keys = with(shortcut, Shortcut::TapCommandShift, Shortcut::OptionSpace);

            let produced = events(
                &mut keys,
                &[flags_changed(key, held), flags_changed(key, 0)],
            );

            assert_eq!(produced, [ActionTapped(Dictate)], "{shortcut:?}");
        }
    }

    #[test]
    fn left_option_is_not_the_right_option_tap() {
        let mut keys = with(
            Shortcut::TapRightOption,
            Shortcut::TapCommandShift,
            Shortcut::OptionSpace,
        );

        let produced = events(
            &mut keys,
            &[
                flags_changed(LEFT_OPTION, LEFT_OPTION_HELD),
                flags_changed(LEFT_OPTION, 0),
            ],
        );

        assert_eq!(produced, []);
    }

    #[test]
    fn control_shift_space_can_be_a_shortcut() {
        let control_shift = LEFT_CONTROL_HELD | LEFT_SHIFT_HELD;
        let mut keys = with(
            Shortcut::TapCommand,
            Shortcut::ControlShiftSpace,
            Shortcut::OptionSpace,
        );

        let decided = decisions(
            &mut keys,
            &[
                down(keycode::SPACE, control_shift),
                up(keycode::SPACE, control_shift),
                down(keycode::SPACE, LEFT_OPTION_HELD),
            ],
        );

        assert_eq!(
            decided,
            [
                KeyDecision {
                    event: Some(ActionTapped(DictateInEnglish)),
                    consume: true,
                },
                KeyDecision::SWALLOW,
                KeyDecision {
                    event: Some(ActionTapped(EditSelection)),
                    consume: true,
                },
            ]
        );
    }

    #[test]
    fn a_chord_matching_two_shortcuts_fires_neither() {
        let mut keys = with(
            Shortcut::TapCommand,
            Shortcut::TapRightCommand,
            Shortcut::OptionSpace,
        );

        let produced = events(
            &mut keys,
            &[
                flags_changed(keycode::RIGHT_COMMAND, RIGHT_COMMAND_HELD),
                flags_changed(keycode::RIGHT_COMMAND, 0),
            ],
        );

        assert_eq!(produced, []);
    }

    #[test]
    fn reconfigured_shortcuts_take_effect_immediately() {
        let mut keys = defaults();
        keys.reconfigure(&ShortcutConfig {
            shortcuts: ActionShortcuts {
                dictate: Shortcut::TapRightOption,
                english: Shortcut::TapCommandShift,
                edit_selection: Shortcut::ControlShiftSpace,
            },
        });

        let mut inputs = command_tap().to_vec();
        inputs.extend([
            down(keycode::SPACE, LEFT_OPTION_HELD),
            flags_changed(LEFT_OPTION, 0),
            flags_changed(keycode::RIGHT_OPTION, RIGHT_OPTION_HELD),
            flags_changed(keycode::RIGHT_OPTION, 0),
        ]);
        let produced = events(&mut keys, &inputs);

        assert_eq!(produced, [ActionTapped(Dictate)]);
    }

    #[test]
    fn reconfiguring_mid_combo_still_swallows_the_space_release() {
        let mut keys = defaults();
        keys.on_key(down(keycode::SPACE, LEFT_OPTION_HELD));
        keys.reconfigure(&ShortcutConfig::default());

        let decided = keys.on_key(up(keycode::SPACE, LEFT_OPTION_HELD));

        assert_eq!(decided, KeyDecision::SWALLOW);
    }
}
