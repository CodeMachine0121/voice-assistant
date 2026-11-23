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
//!   A chord also does not count when it was held longer than [`MAX_TAP_HOLD`],
//!   when Secure Event Input was on (a password field hides keys from the tap),
//!   or when the frontmost app changed meanwhile (Command + Tab is never seen by
//!   the tap). Those last two facts come from [`Surroundings`], asked only when a
//!   chord starts and when it could fire.
//! * **Combos** (Space with exact modifiers). The Space press fires at once and
//!   is swallowed together with its repeats and its release, so no space is typed.
use std::time::Duration;
use voice_core::{Action, ActionShortcuts, Settings, Shortcut, ShortcutEvent};

/// The longest a modifier chord may be held and still count as a tap.
pub const MAX_TAP_HOLD: Duration = Duration::from_secs(1);

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
    /// A mouse button went down, or the user scrolled or made a trackpad gesture.
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
    /// When the event happened, on a clock that only moves forward (time since boot).
    pub timestamp: Duration,
}

/// What the interpreter needs to know about the system around the keyboard.
pub trait Surroundings {
    /// Secure Event Input is on: a password field hides key presses from the tap.
    fn secure_input(&self) -> bool;
    /// Process id of the frontmost application, `None` when unknown.
    fn frontmost_app(&self) -> Option<i32>;
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
    /// When the first modifier of the chord went down.
    chord_started: Duration,
    /// Secure Event Input was on when the chord started.
    chord_secure: bool,
    /// The frontmost app when the chord started.
    chord_app: Option<i32>,
    /// The Space of a combo is down: its repeats and its release are swallowed.
    swallowing_space: bool,
}

impl KeyInterpreter {
    pub fn new(config: &ShortcutConfig) -> Self {
        Self {
            triggers: triggers(&config.shortcuts),
            chord: 0,
            interrupted: false,
            chord_started: Duration::ZERO,
            chord_secure: false,
            chord_app: None,
            swallowing_space: false,
        }
    }

    /// Switches to new shortcuts. Keys already down keep their meaning: a chord in
    /// progress is judged by the new shortcuts on release, a swallowed Space stays swallowed.
    pub fn reconfigure(&mut self, config: &ShortcutConfig) {
        self.triggers = triggers(&config.shortcuts);
    }

    pub fn on_key(&mut self, input: KeyInput, around: &dyn Surroundings) -> KeyDecision {
        match input.kind {
            KeyKind::FlagsChanged => KeyDecision {
                event: self.modifiers_changed(&input, around),
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

    fn modifiers_changed(
        &mut self,
        input: &KeyInput,
        around: &dyn Surroundings,
    ) -> Option<ShortcutEvent> {
        let held = input.flags & flag::PHYSICAL_MODIFIERS;
        if held != 0 {
            if self.chord == 0 {
                self.chord_started = input.timestamp;
                self.chord_secure = around.secure_input();
                self.chord_app = around.frontmost_app();
            }
            self.chord |= held;
            return None;
        }
        let (chord, interrupted) = (self.chord, self.interrupted);
        self.chord = 0;
        self.interrupted = false;
        let held_for = input.timestamp.saturating_sub(self.chord_started);
        if chord == 0 || interrupted || held_for > MAX_TAP_HOLD {
            return None;
        }
        let mut tapped = self
            .triggers
            .iter()
            .filter(|(_, trigger)| trigger.tapped(chord));
        let action = match (tapped.next(), tapped.next()) {
            (Some((action, _)), None) => *action,
            _ => return None,
        };
        if self.chord_secure || around.secure_input() || around.frontmost_app() != self.chord_app {
            return None;
        }
        Some(ShortcutEvent::ActionTapped(action))
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

    /// A quiet system: no secure input, the same app in front all the time.
    struct Calm;

    impl Surroundings for Calm {
        fn secure_input(&self) -> bool {
            false
        }
        fn frontmost_app(&self) -> Option<i32> {
            Some(100)
        }
    }

    fn feed(keys: &mut KeyInterpreter, input: KeyInput) -> KeyDecision {
        keys.on_key(input, &Calm)
    }

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
            timestamp: Duration::ZERO,
        }
    }

    fn down(keycode: u16, flags: u64) -> KeyInput {
        KeyInput {
            kind: KeyKind::Down,
            keycode,
            flags,
            autorepeat: false,
            timestamp: Duration::ZERO,
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
            timestamp: Duration::ZERO,
        }
    }

    fn events(keys: &mut KeyInterpreter, inputs: &[KeyInput]) -> Vec<ShortcutEvent> {
        inputs
            .iter()
            .filter_map(|input| feed(keys, *input).event)
            .collect()
    }

    fn decisions(keys: &mut KeyInterpreter, inputs: &[KeyInput]) -> Vec<KeyDecision> {
        inputs.iter().map(|input| feed(keys, *input)).collect()
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
        feed(&mut keys, click(0));
        feed(&mut keys, down(KEY_E, 0));

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

        let decided = feed(&mut keys, down(keycode::SPACE, RIGHT_OPTION_HELD));

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

        let decided = feed(&mut keys, down(keycode::ESCAPE, 0));

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
        feed(&mut keys, down(keycode::SPACE, LEFT_OPTION_HELD));
        keys.reconfigure(&ShortcutConfig::default());

        let decided = feed(&mut keys, up(keycode::SPACE, LEFT_OPTION_HELD));

        assert_eq!(decided, KeyDecision::SWALLOW);
    }

    /// A system whose answers change between the start of a chord and its release.
    struct Changing {
        secure: [bool; 2],
        app: [Option<i32>; 2],
        asked: std::cell::Cell<usize>,
    }

    impl Changing {
        fn new(secure: [bool; 2], app: [Option<i32>; 2]) -> Self {
            Self {
                secure,
                app,
                asked: std::cell::Cell::new(0),
            }
        }

        /// 0 before the first frontmost-app question, 1 after it.
        fn phase(&self) -> usize {
            self.asked.get().min(1)
        }
    }

    impl Surroundings for Changing {
        fn secure_input(&self) -> bool {
            self.secure[self.phase()]
        }
        fn frontmost_app(&self) -> Option<i32> {
            let app = self.app[self.phase()];
            self.asked.set(self.asked.get() + 1);
            app
        }
    }

    fn at(input: KeyInput, millis: u64) -> KeyInput {
        KeyInput {
            timestamp: Duration::from_millis(millis),
            ..input
        }
    }

    fn timed_command_tap(pressed: u64, released: u64) -> [KeyInput; 2] {
        let [press, release] = command_tap();
        [at(press, pressed), at(release, released)]
    }

    fn tap_with(around: &dyn Surroundings) -> Vec<ShortcutEvent> {
        let mut keys = defaults();
        command_tap()
            .iter()
            .filter_map(|input| keys.on_key(*input, around).event)
            .collect()
    }

    #[test]
    fn a_tap_held_up_to_one_second_counts() {
        let mut keys = defaults();

        let produced = events(&mut keys, &timed_command_tap(5_000, 6_000));

        assert_eq!(produced, [ActionTapped(Dictate)]);
    }

    #[test]
    fn a_chord_held_longer_than_one_second_is_not_a_tap() {
        let mut keys = defaults();

        let produced = events(&mut keys, &timed_command_tap(5_000, 6_001));

        assert_eq!(produced, []);
    }

    #[test]
    fn the_hold_is_timed_from_the_first_modifier_of_the_chord() {
        let both = LEFT_COMMAND_HELD | LEFT_SHIFT_HELD;
        let mut keys = defaults();

        let produced = events(
            &mut keys,
            &[
                at(flags_changed(LEFT_COMMAND, LEFT_COMMAND_HELD), 0),
                at(flags_changed(LEFT_SHIFT, both), 900),
                at(flags_changed(LEFT_SHIFT, LEFT_COMMAND_HELD), 1_000),
                at(flags_changed(LEFT_COMMAND, 0), 1_100),
            ],
        );

        assert_eq!(produced, []);
    }

    #[test]
    fn a_long_hold_does_not_spoil_the_next_tap() {
        let mut keys = defaults();
        events(&mut keys, &timed_command_tap(0, 3_000));

        let produced = events(&mut keys, &timed_command_tap(4_000, 4_200));

        assert_eq!(produced, [ActionTapped(Dictate)]);
    }

    #[test]
    fn no_tap_fires_while_secure_input_is_on() {
        for secure in [[true, true], [true, false], [false, true]] {
            let around = Changing::new(secure, [Some(100), Some(100)]);

            let produced = tap_with(&around);

            assert_eq!(produced, [], "secure input {secure:?}");
        }
    }

    #[test]
    fn no_tap_fires_when_the_frontmost_app_changed_meanwhile() {
        let around = Changing::new([false, false], [Some(100), Some(200)]);

        let produced = tap_with(&around);

        assert_eq!(produced, []);
    }

    #[test]
    fn a_tap_fires_when_the_same_app_stays_in_front() {
        let around = Changing::new([false, false], [Some(100), Some(100)]);

        let produced = tap_with(&around);

        assert_eq!(produced, [ActionTapped(Dictate)]);
    }

    #[test]
    fn the_frontmost_app_is_asked_only_when_a_chord_starts_and_could_fire() {
        let around = Changing::new([false, false], [Some(100), Some(100)]);
        let mut keys = defaults();
        let inputs = [
            down(KEY_E, 0),
            up(KEY_E, 0),
            click(0),
            flags_changed(LEFT_COMMAND, LEFT_COMMAND_HELD),
            flags_changed(LEFT_SHIFT, LEFT_COMMAND_HELD | LEFT_SHIFT_HELD),
            flags_changed(LEFT_SHIFT, LEFT_COMMAND_HELD),
            flags_changed(LEFT_COMMAND, 0),
            flags_changed(LEFT_OPTION, LEFT_OPTION_HELD),
            flags_changed(LEFT_OPTION, 0),
        ];

        for input in inputs {
            keys.on_key(input, &around);
        }

        // Command + Shift: start and release; Option alone: start only (it matches nothing).
        assert_eq!(around.asked.get(), 3);
    }
}
