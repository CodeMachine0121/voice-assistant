//! Pure keyboard interpretation: raw key and pointer events in, shortcut events out.
//!
//! Nothing here talks to the OS, so every mapping rule is unit-tested. The event
//! tap in `shortcuts` only decodes `CGEvent`s into [`KeyInput`] and acts on
//! whatever [`KeyInterpreter::on_key`] decides.
//!
//! The shortcuts are fixed (see [`Shortcut`]) and use the right-hand modifier keys
//! only. Each modifier key has its own device-dependent bit in the event flags, so
//! the left and the right key are told apart; a left key, alone or mixed in, never
//! triggers anything. Two kinds of shortcuts exist:
//!
//! * **Taps** (right Command, or right Command + right Shift, pressed and released
//!   on their own). The interpreter remembers every physical modifier pressed since
//!   the moment all modifiers were last up (the *chord*) and whether anything else
//!   happened meanwhile — a key press or a mouse click. When the last modifier goes
//!   up, a clean chord that is exactly a tap shortcut fires its action. So right
//!   Command that becomes right Command + right Shift fires only the latter, and
//!   Command + C or Command-clicking never starts anything.
//!   A chord also does not count when it was held longer than [`MAX_TAP_HOLD`],
//!   when Secure Event Input was on (a password field hides keys from the tap),
//!   or when the frontmost app changed meanwhile (Command + Tab is never seen by
//!   the tap). Those last two facts come from [`Surroundings`], asked only when a
//!   chord starts and when it could fire.
//! * **Combos** (right Option + Space, with no other modifier held). The Space press
//!   fires at once and is swallowed together with its repeats and its release, so
//!   no space or other character is typed.
use std::time::Duration;
use voice_core::{Action, Shortcut, ShortcutEvent};

/// The longest a modifier chord may be held and still count as a tap.
pub const MAX_TAP_HOLD: Duration = Duration::from_secs(1);

/// Virtual key codes (Carbon `kVK_*`) the interpreter cares about.
pub mod keycode {
    pub const SPACE: u16 = 49;
    pub const ESCAPE: u16 = 53;
    pub const RIGHT_COMMAND: u16 = 54;
    pub const RIGHT_SHIFT: u16 = 60;
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

/// How a shortcut is physically triggered, as the physical modifier keys held
/// (one device-dependent bit per key, see [`flag::PHYSICAL_MODIFIERS`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trigger {
    /// Exactly these modifier keys pressed and released on their own.
    Tap(u64),
    /// Space while exactly these modifier keys are held.
    Combo(u64),
}

fn trigger(shortcut: Shortcut) -> Trigger {
    match shortcut {
        Shortcut::TapRightCommand => Trigger::Tap(flag::RIGHT_COMMAND),
        Shortcut::TapRightCommandRightShift => {
            Trigger::Tap(flag::RIGHT_COMMAND | flag::RIGHT_SHIFT)
        }
        Shortcut::RightOptionSpace => Trigger::Combo(flag::RIGHT_OPTION),
    }
}

impl Trigger {
    fn tapped(self, chord: u64) -> bool {
        self == Trigger::Tap(chord)
    }

    fn combo_pressed(self, input: &KeyInput) -> bool {
        self == Trigger::Combo(input.flags & flag::PHYSICAL_MODIFIERS)
    }
}

/// Every shortcut with the action it triggers.
fn triggers() -> [(Action, Trigger); 3] {
    Shortcut::ALL.map(|shortcut| (shortcut.action(), trigger(shortcut)))
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

impl Default for KeyInterpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyInterpreter {
    pub fn new() -> Self {
        Self {
            triggers: triggers(),
            chord: 0,
            interrupted: false,
            chord_started: Duration::ZERO,
            chord_secure: false,
            chord_app: None,
            swallowing_space: false,
        }
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
        let (action, _) = self
            .triggers
            .iter()
            .find(|(_, trigger)| trigger.tapped(chord))?;
        let action = *action;
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
    const RIGHT_COMMAND: u16 = keycode::RIGHT_COMMAND;
    const RIGHT_SHIFT: u16 = keycode::RIGHT_SHIFT;
    const RIGHT_OPTION: u16 = keycode::RIGHT_OPTION;

    // The flags while a key is held: the device-independent bit plus the key's own.
    const L_CMD: u64 = flag::COMMAND | flag::LEFT_COMMAND;
    const R_CMD: u64 = flag::COMMAND | flag::RIGHT_COMMAND;
    const L_SHIFT: u64 = flag::SHIFT | flag::LEFT_SHIFT;
    const R_SHIFT: u64 = flag::SHIFT | flag::RIGHT_SHIFT;
    const L_OPT: u64 = flag::OPTION | flag::LEFT_OPTION;
    const R_OPT: u64 = flag::OPTION | flag::RIGHT_OPTION;
    const L_CTRL: u64 = flag::CONTROL | flag::LEFT_CONTROL;

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

    fn swallowed(action: Action) -> KeyDecision {
        KeyDecision {
            event: Some(ActionTapped(action)),
            consume: true,
        }
    }

    /// Pressing and releasing one modifier key alone.
    fn tap(key: u16, held: u64) -> [KeyInput; 2] {
        [flags_changed(key, held), flags_changed(key, 0)]
    }

    fn right_command_tap() -> [KeyInput; 2] {
        tap(RIGHT_COMMAND, R_CMD)
    }

    /// Command then Shift down, then Shift up first, then Command.
    fn command_shift_tap(command: (u16, u64), shift: (u16, u64)) -> [KeyInput; 4] {
        let ((cmd_key, cmd), (shift_key, sh)) = (command, shift);
        [
            flags_changed(cmd_key, cmd),
            flags_changed(shift_key, cmd | sh),
            flags_changed(shift_key, cmd),
            flags_changed(cmd_key, 0),
        ]
    }

    /// Space pressed and released while `held` is down, with the modifier around it.
    fn option_space(key: u16, held: u64) -> [KeyInput; 4] {
        [
            flags_changed(key, held),
            down(keycode::SPACE, held),
            up(keycode::SPACE, held),
            flags_changed(key, 0),
        ]
    }

    // --- Dictate: tap the right Command key ----------------------------------

    #[test]
    fn tapping_right_command_dictates_on_release() {
        let mut keys = KeyInterpreter::new();

        let decided = decisions(&mut keys, &right_command_tap());

        assert_eq!(decided, [KeyDecision::PASS, fired(Dictate)]);
    }

    #[test]
    fn tapping_left_command_does_nothing() {
        let mut keys = KeyInterpreter::new();

        let produced = events(&mut keys, &tap(LEFT_COMMAND, L_CMD));

        assert_eq!(produced, []);
    }

    #[test]
    fn right_command_with_left_command_mixed_in_does_nothing() {
        let mut keys = KeyInterpreter::new();
        let both = R_CMD | flag::LEFT_COMMAND;

        let produced = events(
            &mut keys,
            &[
                flags_changed(RIGHT_COMMAND, R_CMD),
                flags_changed(LEFT_COMMAND, both),
                flags_changed(LEFT_COMMAND, R_CMD),
                flags_changed(RIGHT_COMMAND, 0),
            ],
        );

        assert_eq!(produced, []);
    }

    #[test]
    fn right_command_c_does_not_dictate() {
        let mut keys = KeyInterpreter::new();

        let produced = events(
            &mut keys,
            &[
                flags_changed(RIGHT_COMMAND, R_CMD),
                down(KEY_C, R_CMD),
                up(KEY_C, R_CMD),
                flags_changed(RIGHT_COMMAND, 0),
            ],
        );

        assert_eq!(produced, []);
    }

    #[test]
    fn right_command_click_does_not_dictate() {
        let mut keys = KeyInterpreter::new();

        let produced = events(
            &mut keys,
            &[
                flags_changed(RIGHT_COMMAND, R_CMD),
                click(R_CMD),
                flags_changed(RIGHT_COMMAND, 0),
            ],
        );

        assert_eq!(produced, []);
    }

    #[test]
    fn a_click_without_modifiers_does_not_spoil_the_next_tap() {
        let mut keys = KeyInterpreter::new();
        feed(&mut keys, click(0));
        feed(&mut keys, down(KEY_E, 0));

        let produced = events(&mut keys, &right_command_tap());

        assert_eq!(produced, [ActionTapped(Dictate)]);
    }

    #[test]
    fn the_tap_works_again_after_a_command_shortcut() {
        let mut keys = KeyInterpreter::new();
        events(
            &mut keys,
            &[
                flags_changed(RIGHT_COMMAND, R_CMD),
                down(KEY_C, R_CMD),
                flags_changed(RIGHT_COMMAND, 0),
            ],
        );

        let produced = events(&mut keys, &right_command_tap());

        assert_eq!(produced, [ActionTapped(Dictate)]);
    }

    // --- Dictate in English: right Command + right Shift ---------------------

    #[test]
    fn right_command_and_right_shift_dictate_in_english_only() {
        let mut keys = KeyInterpreter::new();

        let produced = events(
            &mut keys,
            &command_shift_tap((RIGHT_COMMAND, R_CMD), (RIGHT_SHIFT, R_SHIFT)),
        );

        assert_eq!(produced, [ActionTapped(DictateInEnglish)], "not Dictate");
    }

    #[test]
    fn english_fires_whichever_right_key_goes_down_or_up_first() {
        let both = R_CMD | R_SHIFT;
        for inputs in [
            // Shift first, Command released first.
            [
                flags_changed(RIGHT_SHIFT, R_SHIFT),
                flags_changed(RIGHT_COMMAND, both),
                flags_changed(RIGHT_COMMAND, R_SHIFT),
                flags_changed(RIGHT_SHIFT, 0),
            ],
            // Command first, Command released first.
            [
                flags_changed(RIGHT_COMMAND, R_CMD),
                flags_changed(RIGHT_SHIFT, both),
                flags_changed(RIGHT_COMMAND, R_SHIFT),
                flags_changed(RIGHT_SHIFT, 0),
            ],
        ] {
            let mut keys = KeyInterpreter::new();

            let produced = events(&mut keys, &inputs);

            assert_eq!(produced, [ActionTapped(DictateInEnglish)]);
        }
    }

    #[test]
    fn command_shift_with_a_left_key_does_nothing() {
        for (command, shift) in [
            ((LEFT_COMMAND, L_CMD), (LEFT_SHIFT, L_SHIFT)),
            ((LEFT_COMMAND, L_CMD), (RIGHT_SHIFT, R_SHIFT)),
            ((RIGHT_COMMAND, R_CMD), (LEFT_SHIFT, L_SHIFT)),
        ] {
            let mut keys = KeyInterpreter::new();

            let produced = events(&mut keys, &command_shift_tap(command, shift));

            assert_eq!(produced, [], "{command:?} {shift:?}");
        }
    }

    #[test]
    fn right_command_shift_with_a_letter_does_nothing() {
        let both = R_CMD | R_SHIFT;
        let mut keys = KeyInterpreter::new();

        let produced = events(
            &mut keys,
            &[
                flags_changed(RIGHT_COMMAND, R_CMD),
                flags_changed(RIGHT_SHIFT, both),
                down(KEY_E, both),
                flags_changed(RIGHT_SHIFT, R_CMD),
                flags_changed(RIGHT_COMMAND, 0),
            ],
        );

        assert_eq!(produced, []);
    }

    #[test]
    fn chords_that_match_no_shortcut_do_nothing() {
        for (key, held) in [
            (LEFT_SHIFT, L_SHIFT),
            (RIGHT_SHIFT, R_SHIFT),
            (LEFT_OPTION, L_OPT),
            (RIGHT_OPTION, R_OPT),
            (LEFT_CONTROL, L_CTRL),
            (keycode::FUNCTION, flag::FUNCTION),
            (RIGHT_COMMAND, R_CMD | L_OPT),
            (RIGHT_COMMAND, R_CMD | R_OPT),
            (RIGHT_COMMAND, R_CMD | R_SHIFT | flag::LEFT_SHIFT),
            (RIGHT_COMMAND, R_CMD | R_SHIFT | L_CTRL),
        ] {
            let mut keys = KeyInterpreter::new();

            let produced = events(&mut keys, &tap(key, held));

            assert_eq!(produced, [], "chord {held:#x}");
        }
    }

    // --- Edit selection: right Option + Space --------------------------------

    #[test]
    fn right_option_space_edits_the_selection_and_no_space_is_typed() {
        let mut keys = KeyInterpreter::new();

        let decided = decisions(
            &mut keys,
            &[
                flags_changed(RIGHT_OPTION, R_OPT),
                down(keycode::SPACE, R_OPT),
                repeat(keycode::SPACE, R_OPT),
                up(keycode::SPACE, R_OPT),
                flags_changed(RIGHT_OPTION, 0),
            ],
        );

        assert_eq!(
            decided,
            [
                KeyDecision::PASS,
                swallowed(EditSelection),
                KeyDecision::SWALLOW,
                KeyDecision::SWALLOW,
                KeyDecision::PASS,
            ]
        );
    }

    #[test]
    fn left_option_space_is_typed_as_usual() {
        let mut keys = KeyInterpreter::new();

        let decided = decisions(&mut keys, &option_space(LEFT_OPTION, L_OPT));

        assert_eq!(decided, [KeyDecision::PASS; 4]);
    }

    #[test]
    fn space_with_both_options_or_other_modifiers_passes_through() {
        for flags in [
            0,
            L_OPT,
            R_OPT | flag::LEFT_OPTION,
            R_OPT | L_SHIFT,
            R_OPT | R_SHIFT,
            R_OPT | R_CMD,
            R_OPT | L_CTRL,
            L_SHIFT,
            L_CTRL,
        ] {
            let mut keys = KeyInterpreter::new();

            let decided = decisions(
                &mut keys,
                &[down(keycode::SPACE, flags), up(keycode::SPACE, flags)],
            );

            assert_eq!(decided, [KeyDecision::PASS; 2], "flags {flags:#x}");
        }
    }

    #[test]
    fn space_released_after_right_option_is_still_swallowed() {
        let mut keys = KeyInterpreter::new();

        let decided = decisions(
            &mut keys,
            &[
                flags_changed(RIGHT_OPTION, R_OPT),
                down(keycode::SPACE, R_OPT),
                flags_changed(RIGHT_OPTION, 0),
                up(keycode::SPACE, 0),
            ],
        );

        assert_eq!(
            decided.iter().map(|d| d.consume).collect::<Vec<_>>(),
            [false, true, false, true]
        );
    }

    #[test]
    fn plain_space_after_a_combo_is_typed() {
        let mut keys = KeyInterpreter::new();
        decisions(
            &mut keys,
            &[down(keycode::SPACE, R_OPT), up(keycode::SPACE, R_OPT)],
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
    fn releasing_right_option_after_a_combo_fires_nothing_more() {
        let mut keys = KeyInterpreter::new();

        let produced = events(&mut keys, &option_space(RIGHT_OPTION, R_OPT));

        assert_eq!(produced, [ActionTapped(EditSelection)]);
    }

    #[test]
    fn auto_repeat_of_a_combo_space_does_not_fire_again() {
        let mut keys = KeyInterpreter::new();

        let produced = events(
            &mut keys,
            &[
                down(keycode::SPACE, R_OPT),
                repeat(keycode::SPACE, R_OPT),
                repeat(keycode::SPACE, R_OPT),
            ],
        );

        assert_eq!(produced, [ActionTapped(EditSelection)]);
    }

    // --- Escape and unrelated input ------------------------------------------

    #[test]
    fn escape_cancels_and_is_passed_on() {
        let mut keys = KeyInterpreter::new();

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
    fn escape_while_holding_right_command_cancels_and_is_not_a_tap() {
        let mut keys = KeyInterpreter::new();

        let produced = events(
            &mut keys,
            &[
                flags_changed(RIGHT_COMMAND, R_CMD),
                down(keycode::ESCAPE, R_CMD),
                flags_changed(RIGHT_COMMAND, 0),
            ],
        );

        assert_eq!(produced, [EscapePressed]);
    }

    #[test]
    fn auto_repeat_and_release_of_escape_are_ignored() {
        let mut keys = KeyInterpreter::new();

        let decided = decisions(
            &mut keys,
            &[repeat(keycode::ESCAPE, 0), up(keycode::ESCAPE, 0)],
        );

        assert_eq!(decided, [KeyDecision::PASS; 2]);
    }

    #[test]
    fn unrelated_keys_produce_nothing() {
        let mut keys = KeyInterpreter::new();

        let decided = decisions(
            &mut keys,
            &[down(KEY_E, 0), repeat(KEY_E, 0), up(KEY_E, 0), click(0)],
        );

        assert_eq!(decided, [KeyDecision::PASS; 4]);
    }

    #[test]
    fn caps_lock_and_other_flag_changes_without_modifiers_do_nothing() {
        let mut keys = KeyInterpreter::new();

        let produced = events(&mut keys, &[flags_changed(57, 0x0001_0000)]);

        assert_eq!(produced, []);
    }

    // --- Timing and surroundings ---------------------------------------------

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

    fn timed_tap(pressed: u64, released: u64) -> [KeyInput; 2] {
        let [press, release] = right_command_tap();
        [at(press, pressed), at(release, released)]
    }

    fn tap_with(around: &dyn Surroundings) -> Vec<ShortcutEvent> {
        let mut keys = KeyInterpreter::new();
        right_command_tap()
            .iter()
            .filter_map(|input| keys.on_key(*input, around).event)
            .collect()
    }

    #[test]
    fn a_tap_held_up_to_one_second_counts() {
        let mut keys = KeyInterpreter::new();

        let produced = events(&mut keys, &timed_tap(5_000, 6_000));

        assert_eq!(produced, [ActionTapped(Dictate)]);
    }

    #[test]
    fn a_chord_held_longer_than_one_second_is_not_a_tap() {
        let mut keys = KeyInterpreter::new();

        let produced = events(&mut keys, &timed_tap(5_000, 6_001));

        assert_eq!(produced, []);
    }

    #[test]
    fn the_hold_is_timed_from_the_first_modifier_of_the_chord() {
        let both = R_CMD | R_SHIFT;
        let mut keys = KeyInterpreter::new();

        let produced = events(
            &mut keys,
            &[
                at(flags_changed(RIGHT_COMMAND, R_CMD), 0),
                at(flags_changed(RIGHT_SHIFT, both), 900),
                at(flags_changed(RIGHT_SHIFT, R_CMD), 1_000),
                at(flags_changed(RIGHT_COMMAND, 0), 1_100),
            ],
        );

        assert_eq!(produced, []);
    }

    #[test]
    fn a_long_hold_does_not_spoil_the_next_tap() {
        let mut keys = KeyInterpreter::new();
        events(&mut keys, &timed_tap(0, 3_000));

        let produced = events(&mut keys, &timed_tap(4_000, 4_200));

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
        let mut keys = KeyInterpreter::new();
        let mut inputs = vec![down(KEY_E, 0), up(KEY_E, 0), click(0)];
        inputs.extend(command_shift_tap(
            (RIGHT_COMMAND, R_CMD),
            (RIGHT_SHIFT, R_SHIFT),
        ));
        inputs.extend(tap(LEFT_OPTION, L_OPT));

        for input in inputs {
            keys.on_key(input, &around);
        }

        // Right Command + right Shift: start and release; left Option: start only
        // (it matches nothing).
        assert_eq!(around.asked.get(), 3);
    }
}
