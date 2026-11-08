//! Keyboard translation: iced key events → the framework-neutral types the
//! rest of the app speaks. Two targets — keymap chords (FR9 shortcuts) and the
//! PTY codec's [`TermKey`]/[`KeyMods`] — plus the held-modifier helper for the
//! link-open gesture. Pure functions, no `Shell` coupling.

use iced::keyboard;
use termherd_core::{KeyChord, keymap};
use termherd_pty::{KeyMods, TermKey};

use crate::settings::OptionAsMeta;

/// The keymap chord for a key press (FR9): the key's normalised name plus the
/// modifier bits. `None` for keys we do not bind (so they reach the terminal).
///
/// The number row is matched by **physical position**, so ⌘1…⌘9
/// land on the same keys on every layout — on AZERTY the top-left key reports
/// the character `&` yet the physical code `Digit1`, so it still binds tab 1.
/// Every other key stays character-based (⌘C follows the letter C, not its
/// position), matching how browsers and terminals treat these chords.
pub(super) fn chord_of(
    key: &keyboard::Key,
    physical: &keyboard::key::Physical,
    modifiers: keyboard::Modifiers,
) -> Option<KeyChord> {
    let name = physical_digit_name(physical).or_else(|| key_name(key))?;
    let mut mods = 0u8;
    if modifiers.control() {
        mods |= keymap::MOD_CTRL;
    }
    if modifiers.alt() {
        mods |= keymap::MOD_ALT;
    }
    if modifiers.shift() {
        mods |= keymap::MOD_SHIFT;
    }
    if modifiers.logo() {
        mods |= keymap::MOD_CMD;
    }
    Some(KeyChord::new(name, mods))
}

/// The key event a [`KeyChord`] stands for — the inverse of [`chord_of`].
///
/// This is what lets a chord that arrived as *text* (over the MCP control
/// surface) walk the very same `on_key` ladder a physical press walks, instead
/// of getting a second, shortcut-only dispatch path. So an overlay consumes what
/// it would consume for a human, and `escape` / `enter` reach it — a chord
/// resolved straight to its keymap action could never do that.
///
/// `None` for a chord naming a key no event can carry (`"f13"`, `"capslock"`):
/// no real press resolves such a binding either, so no round trip is owed.
///
/// The number row is rebuilt by *physical* position, mirroring how `chord_of`
/// reads it — otherwise `cmd+1` would come back as the character `1` and fail to
/// resolve on a layout where that key types something else.
pub(super) fn event_of(chord: &KeyChord) -> Option<keyboard::Event> {
    let (key, physical_key) = named_key(&chord.key)
        .map(|named| (keyboard::Key::Named(named), NON_DIGIT))
        .or_else(|| digit_key(&chord.key))
        .or_else(|| character_key(&chord.key))?;
    Some(keyboard::Event::KeyPressed {
        modified_key: key.clone(),
        // The chord's character *is* the text it types, so an unbound character
        // chord falls through to the terminal typing that character. Chords are
        // lowercased, so this types lower case; typing arbitrary text is
        // `run_in_session`'s job, not this tool's.
        text: match &key {
            keyboard::Key::Character(c) => Some(c.clone()),
            keyboard::Key::Named(_) | keyboard::Key::Unidentified => None,
        },
        key,
        physical_key,
        location: keyboard::Location::Standard,
        modifiers: modifiers_of(chord.mods),
        repeat: false,
    })
}

/// A physical key that is not on the number row, for the character and named
/// keys whose chord name comes from the key itself rather than its position.
const NON_DIGIT: keyboard::key::Physical =
    keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified);

/// A named key as the three sides of the shell see it: the name a chord spells
/// it by, the key iced reports, and the key the terminal codec encodes.
#[derive(Debug, Clone, Copy)]
struct NamedKey {
    name: &'static str,
    key: keyboard::key::Named,
    term: TermKey,
}

const fn named(name: &'static str, key: keyboard::key::Named, term: TermKey) -> NamedKey {
    NamedKey { name, key, term }
}

/// The named keys a chord can spell and a terminal receives. The one table
/// every direction reads, so a key nameable on a real press is nameable over
/// MCP and the reverse, and a nameable key bound to nothing reaches the
/// terminal. A named key missing here reaches neither.
const NAMED_KEYS: &[NamedKey] = {
    use keyboard::key::Named;
    &[
        named("tab", Named::Tab, TermKey::Tab),
        named("enter", Named::Enter, TermKey::Enter),
        named("escape", Named::Escape, TermKey::Escape),
        // Arrow keys carry chord names so bindings like `mod+up` (scroll) and
        // `mod+shift+left` (pane focus) resolve, instead of leaking the raw
        // cursor sequence to the shell.
        named("up", Named::ArrowUp, TermKey::Up),
        named("down", Named::ArrowDown, TermKey::Down),
        named("left", Named::ArrowLeft, TermKey::Left),
        named("right", Named::ArrowRight, TermKey::Right),
        // Home / End name the scroll-to-top / bottom chords off macOS.
        named("home", Named::Home, TermKey::Home),
        named("end", Named::End, TermKey::End),
        named("pageup", Named::PageUp, TermKey::PageUp),
        named("pagedown", Named::PageDown, TermKey::PageDown),
        named("insert", Named::Insert, TermKey::Insert),
        named("delete", Named::Delete, TermKey::Delete),
        named("backspace", Named::Backspace, TermKey::Backspace),
        named("space", Named::Space, TermKey::Space),
        named("f1", Named::F1, TermKey::F1),
        named("f2", Named::F2, TermKey::F2),
        named("f3", Named::F3, TermKey::F3),
        named("f4", Named::F4, TermKey::F4),
        named("f5", Named::F5, TermKey::F5),
        named("f6", Named::F6, TermKey::F6),
        named("f7", Named::F7, TermKey::F7),
        named("f8", Named::F8, TermKey::F8),
        named("f9", Named::F9, TermKey::F9),
        named("f10", Named::F10, TermKey::F10),
        named("f11", Named::F11, TermKey::F11),
        named("f12", Named::F12, TermKey::F12),
    ]
};

/// The table row for an iced named key, or `None` for a key outside it.
fn row_of(key: keyboard::key::Named) -> Option<&'static NamedKey> {
    NAMED_KEYS.iter().find(|row| row.key == key)
}

/// The named key a chord name spells, or `None` when it names a character.
fn named_key(name: &str) -> Option<keyboard::key::Named> {
    NAMED_KEYS
        .iter()
        .find_map(|row| (row.name == name).then_some(row.key))
}

/// A single-digit chord name as a number-row key: the character it types plus
/// the physical code [`physical_digit_name`] reads back.
fn digit_key(name: &str) -> Option<(keyboard::Key, keyboard::key::Physical)> {
    use keyboard::key::{Code, Physical};
    let code = match name {
        "0" => Code::Digit0,
        "1" => Code::Digit1,
        "2" => Code::Digit2,
        "3" => Code::Digit3,
        "4" => Code::Digit4,
        "5" => Code::Digit5,
        "6" => Code::Digit6,
        "7" => Code::Digit7,
        "8" => Code::Digit8,
        "9" => Code::Digit9,
        _ => return None,
    };
    Some((keyboard::Key::Character(name.into()), Physical::Code(code)))
}

/// A one-character chord name as a character key. Longer names are rejected:
/// [`key_name`] only ever produces one character, so a multi-character name that
/// [`named_key`] did not claim can round-trip through nothing.
fn character_key(name: &str) -> Option<(keyboard::Key, keyboard::key::Physical)> {
    let mut chars = name.chars();
    let ch = chars.next()?;
    chars
        .next()
        .is_none()
        .then(|| (keyboard::Key::Character(ch.to_string().into()), NON_DIGIT))
}

/// The iced modifier state for a chord's [`keymap`] modifier bits — the inverse
/// of the bit-setting half of [`chord_of`].
fn modifiers_of(mods: u8) -> keyboard::Modifiers {
    let mut modifiers = keyboard::Modifiers::default();
    if mods & keymap::MOD_CTRL != 0 {
        modifiers |= keyboard::Modifiers::CTRL;
    }
    if mods & keymap::MOD_ALT != 0 {
        modifiers |= keyboard::Modifiers::ALT;
    }
    if mods & keymap::MOD_SHIFT != 0 {
        modifiers |= keyboard::Modifiers::SHIFT;
    }
    if mods & keymap::MOD_CMD != 0 {
        modifiers |= keyboard::Modifiers::LOGO;
    }
    modifiers
}

/// The digit `"0"`…`"9"` for a number-row key by physical position, or `None`
/// for any other key. Layout-independent: the same physical keys carry these
/// names on QWERTY, AZERTY, QWERTZ, … so number-row shortcuts are universal —
/// 1–9 for the tab jumps, 0 for zoom-reset, which would
/// otherwise need Shift on AZERTY (where unshifted 0 types `à`).
fn physical_digit_name(physical: &keyboard::key::Physical) -> Option<String> {
    use keyboard::key::{Code, Physical};
    let digit = match physical {
        Physical::Code(Code::Digit0) => '0',
        Physical::Code(Code::Digit1) => '1',
        Physical::Code(Code::Digit2) => '2',
        Physical::Code(Code::Digit3) => '3',
        Physical::Code(Code::Digit4) => '4',
        Physical::Code(Code::Digit5) => '5',
        Physical::Code(Code::Digit6) => '6',
        Physical::Code(Code::Digit7) => '7',
        Physical::Code(Code::Digit8) => '8',
        Physical::Code(Code::Digit9) => '9',
        _ => return None,
    };
    Some(digit.to_string())
}

/// The keymap name of an iced key: a lowercased character, or a named key from
/// [`NAMED_KEYS`]. `None` for keys no shortcut can target.
fn key_name(key: &keyboard::Key) -> Option<String> {
    match key {
        keyboard::Key::Character(c) => c
            .chars()
            .next()
            .map(|ch| ch.to_ascii_lowercase().to_string()),
        keyboard::Key::Named(named) => row_of(*named).map(|row| row.name.to_owned()),
        keyboard::Key::Unidentified => None,
    }
}

/// The character a numpad key typed when NumLock turned it into text. With
/// NumLock on, winit's `key_without_modifiers` reports the *un*-locked key — so
/// numpad `1` arrives as `Named(End)`, `2` as `Named(ArrowDown)`, … — yet the
/// digit/operator is still in `text`. Honouring it here keeps the numpad typing
/// digits instead of moving the cursor. `None` for non-numpad keys, and for
/// numpad keys with no printable single-char text (NumLock off → navigation;
/// Numpad-Enter → its named sequence), which then fall through to [`to_term_key`].
pub(super) fn numpad_char(location: keyboard::Location, text: Option<&str>) -> Option<char> {
    if location != keyboard::Location::Numpad {
        return None;
    }
    let mut chars = text?.chars();
    let ch = chars.next()?;
    (chars.next().is_none() && !ch.is_control()).then_some(ch)
}

/// Map an iced key to the framework-neutral [`TermKey`] the PTY codec speaks
/// (`termherd_pty::key_bytes`). `None` for keys with no terminal sequence, so
/// they reach no PTY. The byte protocol itself lives in the terminal adapter.
pub(super) fn to_term_key(key: &keyboard::Key) -> Option<TermKey> {
    match key {
        keyboard::Key::Character(c) => c.chars().next().map(TermKey::Char),
        keyboard::Key::Named(named) => row_of(*named).map(|row| row.term),
        keyboard::Key::Unidentified => None,
    }
}

/// The PTY codec's view of the held modifiers (Cmd/Super never affects bytes).
pub(super) fn key_mods(m: keyboard::Modifiers) -> KeyMods {
    KeyMods {
        ctrl: m.control(),
        alt: m.alt(),
        shift: m.shift(),
    }
}

/// Which Alt (Option) keys are held, read from the Alt keys' own press and
/// release: a letter's event says Alt is down, never which one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct AltSides {
    left: bool,
    right: bool,
}

/// Alt key events as winit reports them, for tests on both sides of the shell.
#[cfg(test)]
pub(super) mod alt_events {
    use iced::keyboard::key::{Code, Named, Physical};
    use iced::keyboard::{Event, Key, Location, Modifiers};

    fn physical(location: Location) -> Physical {
        Physical::Code(match location {
            Location::Right => Code::AltRight,
            _ => Code::AltLeft,
        })
    }

    pub(in crate::shell) fn alt_down(location: Location) -> Event {
        let key = Key::Named(Named::Alt);
        Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: physical(location),
            location,
            modifiers: Modifiers::ALT,
            text: None,
            repeat: false,
        }
    }

    /// The Alt key at `location` coming up, leaving `modifiers` held.
    pub(in crate::shell) fn alt_up(location: Location, modifiers: Modifiers) -> Event {
        let key = Key::Named(Named::Alt);
        Event::KeyReleased {
            key: key.clone(),
            modified_key: key,
            physical_key: physical(location),
            location,
            modifiers,
        }
    }
}

impl AltSides {
    /// Fold one keyboard event into the held sides.
    pub(super) fn track(&mut self, event: &keyboard::Event) {
        use keyboard::key::Named;
        match event {
            keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(Named::Alt),
                location,
                ..
            } => self.set(*location, true),
            keyboard::Event::KeyReleased {
                key: keyboard::Key::Named(Named::Alt),
                location,
                ..
            } => self.set(*location, false),
            // A release lost to a focus change must not leave a side stuck down.
            _ if !event_modifiers(event).alt() => *self = Self::default(),
            _ => {}
        }
    }

    fn set(&mut self, location: keyboard::Location, held: bool) {
        match location {
            keyboard::Location::Left => self.left = held,
            keyboard::Location::Right => self.right = held,
            _ => {}
        }
    }
}

/// Whether a held Alt sends Meta under `option`. An Alt with no known side
/// (an MCP press) counts as either side.
pub(super) fn sends_meta(option: OptionAsMeta, sides: AltSides) -> bool {
    match option {
        OptionAsMeta::None => false,
        OptionAsMeta::Left => sides.left || !sides.right,
        OptionAsMeta::Right => sides.right || !sides.left,
        OptionAsMeta::Both => true,
    }
}

/// The modifier state carried by a keyboard event, if it carries one. Used to
/// keep the link-open modifier tracked across press / release / change.
pub(super) fn event_modifiers(event: &keyboard::Event) -> keyboard::Modifiers {
    match event {
        keyboard::Event::KeyPressed { modifiers, .. }
        | keyboard::Event::KeyReleased { modifiers, .. }
        | keyboard::Event::ModifiersChanged(modifiers) => *modifiers,
    }
}

#[cfg(test)]
mod tests {
    use super::alt_events::{alt_down, alt_up};
    use super::*;
    use iced::keyboard::key::{Code, Named, Physical};
    use iced::keyboard::{Key, Location, Modifiers};
    use termherd_core::keymap::{Platform, default_bindings};

    /// The chord a synthesised event reads back as, or `None` when the chord
    /// could not be synthesised at all — the whole round trip in one call.
    fn round_trip(chord: &KeyChord) -> Option<KeyChord> {
        let keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        } = event_of(chord)?
        else {
            return None;
        };
        chord_of(&key, &physical_key, modifiers)
    }

    #[test]
    fn to_term_key_maps_characters_and_named_keys() {
        // Characters carry their base char; the named keys the codec handles
        // map to their `TermKey`.
        assert_eq!(
            to_term_key(&Key::Character("a".into())),
            Some(TermKey::Char('a'))
        );
        assert_eq!(to_term_key(&Key::Named(Named::Enter)), Some(TermKey::Enter));
        assert_eq!(to_term_key(&Key::Named(Named::ArrowUp)), Some(TermKey::Up));
        assert_eq!(
            to_term_key(&Key::Named(Named::PageDown)),
            Some(TermKey::PageDown)
        );
        assert_eq!(to_term_key(&Key::Named(Named::F2)), Some(TermKey::F2));
        assert_eq!(
            to_term_key(&Key::Named(Named::Insert)),
            Some(TermKey::Insert)
        );
        // Keys with no terminal sequence reach no PTY.
        assert_eq!(to_term_key(&Key::Named(Named::CapsLock)), None);
        assert_eq!(to_term_key(&Key::Unidentified), None);
    }

    #[test]
    fn key_mods_drops_the_platform_command_key() {
        assert_eq!(
            key_mods(Modifiers::CTRL | Modifiers::SHIFT),
            KeyMods {
                ctrl: true,
                alt: false,
                shift: true
            }
        );
        // Cmd / Super (logo) never affects terminal bytes.
        assert_eq!(key_mods(Modifiers::LOGO), KeyMods::default());
    }

    const LEFT: AltSides = AltSides {
        left: true,
        right: false,
    };
    const RIGHT: AltSides = AltSides {
        left: false,
        right: true,
    };
    const BOTH: AltSides = AltSides {
        left: true,
        right: true,
    };

    #[test]
    fn sends_meta_follows_the_side_the_setting_names() {
        use OptionAsMeta::{Both, Left, None, Right};
        for (option, sides, meta) in [
            (Left, LEFT, true),
            (Left, RIGHT, false),
            (Right, LEFT, false),
            (Right, RIGHT, true),
            (Both, LEFT, true),
            (Both, RIGHT, true),
            (None, LEFT, false),
            (None, RIGHT, false),
        ] {
            assert_eq!(sends_meta(option, sides), meta, "{option:?} with {sides:?}");
        }
    }

    #[test]
    fn an_alt_with_no_known_side_is_meta_unless_neither_key_is() {
        let unknown = AltSides::default();
        for option in [OptionAsMeta::Left, OptionAsMeta::Right, OptionAsMeta::Both] {
            assert!(sends_meta(option, unknown), "{option:?}");
        }
        assert!(!sends_meta(OptionAsMeta::None, unknown));
    }

    #[test]
    fn alt_sides_follow_each_alt_keys_press_and_release() {
        let mut sides = AltSides::default();
        sides.track(&alt_down(Location::Right));
        assert_eq!(sides, RIGHT);
        sides.track(&alt_down(Location::Left));
        assert_eq!(sides, BOTH);
        sides.track(&alt_up(Location::Right, Modifiers::ALT));
        assert_eq!(sides, LEFT);
    }

    #[test]
    fn any_event_without_alt_held_clears_both_sides() {
        let mut sides = BOTH;
        sides.track(&keyboard::Event::ModifiersChanged(Modifiers::empty()));
        assert_eq!(sides, AltSides::default());
    }

    #[test]
    fn chord_of_builds_keymap_chords_from_key_events() {
        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
        assert_eq!(
            chord_of(&Key::Character("C".into()), &NON_DIGIT, ctrl_shift),
            Some(KeyChord::new("c", keymap::MOD_CTRL | keymap::MOD_SHIFT))
        );
        assert_eq!(
            chord_of(&Key::Named(Named::Tab), &NON_DIGIT, Modifiers::CTRL),
            Some(KeyChord::new("tab", keymap::MOD_CTRL))
        );
        // Arrow keys carry chord names so `mod+up` / `mod+shift+left` resolve
        // instead of leaking the cursor sequence to the terminal.
        assert_eq!(
            chord_of(
                &Key::Named(Named::ArrowRight),
                &NON_DIGIT,
                Modifiers::LOGO | Modifiers::SHIFT
            ),
            Some(KeyChord::new("right", keymap::MOD_CMD | keymap::MOD_SHIFT))
        );
        assert_eq!(
            chord_of(&Key::Named(Named::ArrowUp), &NON_DIGIT, Modifiers::LOGO),
            Some(KeyChord::new("up", keymap::MOD_CMD))
        );
        assert_eq!(
            chord_of(&Key::Named(Named::F2), &NON_DIGIT, Modifiers::default()),
            Some(KeyChord::new("f2", 0))
        );
        // Keys no shortcut targets carry no chord.
        assert_eq!(
            chord_of(
                &Key::Named(Named::CapsLock),
                &NON_DIGIT,
                Modifiers::default()
            ),
            None
        );
    }

    #[test]
    fn number_row_chords_follow_the_physical_key_across_layouts() {
        // AZERTY: the top-left key reports the character `&` but the physical
        // code `Digit1`. ⌘+& must still bind tab 1, so the physical
        // position — not the character — decides the digit.
        assert_eq!(
            chord_of(
                &Key::Character("&".into()),
                &Physical::Code(Code::Digit1),
                Modifiers::LOGO,
            ),
            Some(KeyChord::new("1", keymap::MOD_CMD))
        );
        // QWERTY: character and position agree — same chord, no regression.
        assert_eq!(
            chord_of(
                &Key::Character("1".into()),
                &Physical::Code(Code::Digit1),
                Modifiers::LOGO,
            ),
            Some(KeyChord::new("1", keymap::MOD_CMD))
        );
    }

    #[test]
    fn physical_digit_names_cover_the_number_row() {
        for (code, expected) in [
            (Code::Digit1, Some("1".to_string())),
            (Code::Digit9, Some("9".to_string())),
            // Zero is not a tab shortcut but carries zoom-reset, so it
            // is named too — layout-independently, like the tab digits.
            (Code::Digit0, Some("0".to_string())),
            // A non-digit physical key falls through to the character path.
            (Code::KeyA, None),
        ] {
            assert_eq!(physical_digit_name(&Physical::Code(code)), expected);
        }
    }

    #[test]
    fn numpad_with_numlock_types_its_digit_not_a_navigation_key() {
        // With NumLock on the key reports an un-locked name (handled by `key`),
        // but `text` carries the digit/operator — that is what should be typed.
        assert_eq!(
            numpad_char(keyboard::Location::Numpad, Some("1")),
            Some('1')
        );
        assert_eq!(
            numpad_char(keyboard::Location::Numpad, Some("+")),
            Some('+')
        );
        // Off the numpad the field is ignored — the main row already types via
        // its own `text`, so this must not hijack it.
        assert_eq!(numpad_char(keyboard::Location::Standard, Some("1")), None);
        // No printable single char (NumLock off → navigation; Numpad-Enter →
        // a control char; empty) falls through to the named-key mapping.
        assert_eq!(numpad_char(keyboard::Location::Numpad, None), None);
        assert_eq!(numpad_char(keyboard::Location::Numpad, Some("")), None);
        assert_eq!(numpad_char(keyboard::Location::Numpad, Some("\r")), None);
    }

    // ---- `event_of`: the inverse of `chord_of` (F-mcp-keys) --------------
    //
    // A chord that arrived as text must reach the app as an event the routing
    // ladder cannot tell from a real press. The round trip is the property that
    // pins it: whatever `event_of` builds, `chord_of` must read back unchanged.

    #[test]
    fn every_chord_the_shipped_keymap_can_hold_round_trips() {
        // The defaults are the chords that must work on day one. A chord that
        // does not round-trip is a binding `press_keys` silently cannot reach —
        // exactly the silent-drop this rung exists to avoid.
        // Every platform's, so a Windows-only chord is checked from a Mac.
        for platform in Platform::ALL {
            for (chord, action) in default_bindings(platform) {
                assert_eq!(
                    round_trip(&chord).as_ref(),
                    Some(&chord),
                    "default chord {chord:?} for {action:?} on {platform:?} must round-trip"
                );
            }
        }
    }

    #[test]
    fn the_number_row_round_trips_by_physical_position() {
        // `chord_of` reads digits positionally, so `event_of` must *write* them
        // positionally. A character-only digit event would read back as whatever
        // that key types on the user's layout — on AZERTY, not a digit at all.
        //
        // The physical code has to be asserted per digit, not inferred from the
        // round trip: a digit that fell through to the character path still
        // round-trips here (`chord_of` reads its character when the physical key
        // is not a digit code), so the round trip alone cannot see the bug. A
        // mutation run proved it — deleting nine of the ten arms in `digit_key`
        // survived a version of this test that spot-checked one digit.
        for (digit, code) in [
            ('0', Code::Digit0),
            ('1', Code::Digit1),
            ('2', Code::Digit2),
            ('3', Code::Digit3),
            ('4', Code::Digit4),
            ('5', Code::Digit5),
            ('6', Code::Digit6),
            ('7', Code::Digit7),
            ('8', Code::Digit8),
            ('9', Code::Digit9),
        ] {
            let chord = KeyChord::new(digit.to_string(), keymap::MOD_CMD);
            assert_eq!(round_trip(&chord).as_ref(), Some(&chord));
            let keyboard::Event::KeyPressed { physical_key, .. } =
                event_of(&chord).expect("a digit chord synthesises")
            else {
                unreachable!("event_of builds a key press")
            };
            assert_eq!(
                physical_key,
                Physical::Code(code),
                "`{digit}` must carry its physical code, not just its character"
            );
        }
    }

    #[test]
    fn the_editing_and_function_keys_are_nameable() {
        // Each of these has a terminal sequence, so a chord naming it must reach
        // the keymap on a real press and be synthesisable over `press_keys`.
        let names = [
            "pageup",
            "pagedown",
            "delete",
            "backspace",
            "space",
            "insert",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain((1..=12).map(|n| format!("f{n}")));
        for name in names {
            let chord = KeyChord::new(name.as_str(), 0);
            assert_eq!(round_trip(&chord).as_ref(), Some(&chord), "`{name}`");
        }
    }

    #[test]
    fn every_named_key_round_trips() {
        // Read from the table the code uses, so a row added later is covered.
        for row in NAMED_KEYS {
            let chord = KeyChord::new(row.name, keymap::MOD_SHIFT);
            assert_eq!(round_trip(&chord).as_ref(), Some(&chord), "`{}`", row.name);
        }
    }

    /// Every named key as a real press reports it, with the chord name and the
    /// terminal key it must resolve to, spelled out. The round trips above read
    /// `NAMED_KEYS` in both directions, so two rows with swapped iced keys pass
    /// them; only a list written independently of the table can fail.
    const SPELLED_NAMED_KEYS: [(Named, &str, TermKey); 27] = [
        (Named::Tab, "tab", TermKey::Tab),
        (Named::Enter, "enter", TermKey::Enter),
        (Named::Escape, "escape", TermKey::Escape),
        (Named::ArrowUp, "up", TermKey::Up),
        (Named::ArrowDown, "down", TermKey::Down),
        (Named::ArrowLeft, "left", TermKey::Left),
        (Named::ArrowRight, "right", TermKey::Right),
        (Named::Home, "home", TermKey::Home),
        (Named::End, "end", TermKey::End),
        (Named::PageUp, "pageup", TermKey::PageUp),
        (Named::PageDown, "pagedown", TermKey::PageDown),
        (Named::Insert, "insert", TermKey::Insert),
        (Named::Delete, "delete", TermKey::Delete),
        (Named::Backspace, "backspace", TermKey::Backspace),
        (Named::Space, "space", TermKey::Space),
        (Named::F1, "f1", TermKey::F1),
        (Named::F2, "f2", TermKey::F2),
        (Named::F3, "f3", TermKey::F3),
        (Named::F4, "f4", TermKey::F4),
        (Named::F5, "f5", TermKey::F5),
        (Named::F6, "f6", TermKey::F6),
        (Named::F7, "f7", TermKey::F7),
        (Named::F8, "f8", TermKey::F8),
        (Named::F9, "f9", TermKey::F9),
        (Named::F10, "f10", TermKey::F10),
        (Named::F11, "f11", TermKey::F11),
        (Named::F12, "f12", TermKey::F12),
    ];

    #[test]
    fn a_real_named_key_resolves_to_its_spelled_name_and_terminal_key() {
        for (named, name, term) in SPELLED_NAMED_KEYS {
            let key = Key::Named(named);
            assert_eq!(key_name(&key).as_deref(), Some(name), "{named:?}");
            assert_eq!(to_term_key(&key), Some(term), "{named:?}");
        }
    }

    #[test]
    fn the_spelled_list_covers_every_row_of_the_name_table() {
        // Derived from the table the code reads, so a row added there without
        // a spelled twin fails here rather than going unchecked.
        for row in NAMED_KEYS {
            assert!(
                SPELLED_NAMED_KEYS
                    .iter()
                    .any(|(named, _, _)| *named == row.key),
                "`{}` has no spelled-out row",
                row.name
            );
        }
    }

    #[test]
    fn no_two_rows_share_a_name_or_a_key() {
        // Lookups take the first match, so a second row for the same name or
        // key would be dead in one direction and live in the other.
        for (i, row) in NAMED_KEYS.iter().enumerate() {
            for other in &NAMED_KEYS[i + 1..] {
                assert_ne!(row.name, other.name);
                assert_ne!(row.key, other.key, "`{}` and `{}`", row.name, other.name);
            }
        }
    }

    #[test]
    fn a_chord_no_key_event_can_carry_synthesises_nothing() {
        // `key_name` never produces these, so no real press could resolve such a
        // binding either. Reporting nothing is the faithful answer; inventing an
        // event would give the MCP surface a reach the keyboard lacks.
        for name in ["f13", "capslock", "printscreen"] {
            assert_eq!(
                event_of(&KeyChord::new(name, keymap::MOD_CMD)),
                None,
                "`{name}` names no key an event can carry"
            );
        }
    }

    #[test]
    fn a_character_chord_carries_its_character_as_typed_text() {
        // An unbound character chord falls through to the terminal, which types
        // `text` — so a chord with no text would reach the PTY as nothing.
        let keyboard::Event::KeyPressed { text, .. } =
            event_of(&KeyChord::new("a", 0)).expect("a character chord synthesises")
        else {
            unreachable!("event_of builds a key press")
        };
        assert_eq!(text.as_deref(), Some("a"));
        // A named key types no text of its own — `key_bytes` derives its
        // sequence from the `TermKey`, and inventing text here would double it.
        let keyboard::Event::KeyPressed { text, .. } =
            event_of(&KeyChord::new("enter", 0)).expect("a named chord synthesises")
        else {
            unreachable!("event_of builds a key press")
        };
        assert_eq!(text, None);
    }

    #[test]
    fn modifiers_round_trip_through_every_combination() {
        // All 16 combinations of the four modifier bits, so neither direction can
        // drop or transpose one — a swapped Cmd/Ctrl would make every default
        // binding unreachable on one platform and silently work on the other.
        for mods in 0u8..16 {
            let chord = KeyChord::new("k", mods);
            assert_eq!(
                round_trip(&chord).as_ref(),
                Some(&chord),
                "modifier bits {mods:#06b} must round-trip"
            );
        }
    }

    proptest::proptest! {
        /// Every chord built from a key name the keymap can name and any
        /// modifier set survives the round trip.
        #[test]
        fn arbitrary_nameable_chords_round_trip(
            name in proptest::sample::select(
                ["a", "z", "0", "5", "9", "-", "=", "+"]
                    .into_iter()
                    .chain(NAMED_KEYS.iter().map(|row| row.name))
                    .collect::<Vec<_>>()
            ),
            mods in 0u8..16,
        ) {
            let chord = KeyChord::new(name, mods);
            let read_back = round_trip(&chord);
            proptest::prop_assert_eq!(read_back.as_ref(), Some(&chord));
        }
    }

    #[test]
    fn letters_stay_character_based_not_positional() {
        // ⌘C is the letter C wherever it sits — a non-digit physical key must
        // not hijack the character path.
        assert_eq!(
            chord_of(
                &Key::Character("c".into()),
                &Physical::Code(Code::KeyC),
                Modifiers::LOGO,
            ),
            Some(KeyChord::new("c", keymap::MOD_CMD))
        );
    }

    /// What the default keymap does with a press, built the way iced hands it
    /// over: `key` is winit's `key_without_modifiers`, so Shift never changes
    /// the character it names.
    fn default_action_of(
        platform: keymap::Platform,
        key: &str,
        physical: Code,
        modifiers: Modifiers,
    ) -> Option<termherd_core::Action> {
        let chord = chord_of(
            &Key::Character(key.into()),
            &Physical::Code(physical),
            modifiers,
        )?;
        keymap::default_bindings(platform)
            .into_iter()
            .find_map(|(bound, action)| (bound == chord).then_some(action))
    }

    #[test]
    fn the_tab_menu_chord_is_reachable_from_a_real_press_on_qwerty_and_azerty() {
        use keymap::Platform;
        use termherd_core::Action;
        let cmd_shift = Modifiers::LOGO | Modifiers::SHIFT;
        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
        // QWERTY: M sits on KeyM. AZERTY: the M key is where QWERTY has `;`.
        for physical in [Code::KeyM, Code::Semicolon] {
            assert_eq!(
                default_action_of(Platform::MacOs, "m", physical, cmd_shift),
                Some(Action::OpenTabMenu),
                "{physical:?} on macOS"
            );
            assert_eq!(
                default_action_of(Platform::Linux, "m", physical, ctrl_shift),
                Some(Action::OpenTabMenu),
                "{physical:?} on Windows and Linux"
            );
        }
    }

    #[test]
    fn a_shifted_punctuation_chord_arrives_as_its_unshifted_key() {
        // Why `.` cannot be a default: on AZERTY it is Shift+`;`, and the key
        // a press reports drops the Shift, so Cmd+Shift+`;` names `;`.
        assert_eq!(
            chord_of(
                &Key::Character(";".into()),
                &Physical::Code(Code::Comma),
                Modifiers::LOGO | Modifiers::SHIFT,
            ),
            Some(KeyChord::new(";", keymap::MOD_CMD | keymap::MOD_SHIFT))
        );
    }

    // ---- Which default chords a real press can produce ------------------
    //
    // A layout is its typing keys as iced reports them: the physical code and
    // the character the key types without Shift, which `key` carries whether
    // Shift is held or not. Dead keys are left out: they report no character.

    type Layout = &'static [(Code, &'static str)];

    const QWERTY_US: Layout = &[
        (Code::Backquote, "`"),
        (Code::Digit1, "1"),
        (Code::Digit2, "2"),
        (Code::Digit3, "3"),
        (Code::Digit4, "4"),
        (Code::Digit5, "5"),
        (Code::Digit6, "6"),
        (Code::Digit7, "7"),
        (Code::Digit8, "8"),
        (Code::Digit9, "9"),
        (Code::Digit0, "0"),
        (Code::Minus, "-"),
        (Code::Equal, "="),
        (Code::KeyQ, "q"),
        (Code::KeyW, "w"),
        (Code::KeyE, "e"),
        (Code::KeyR, "r"),
        (Code::KeyT, "t"),
        (Code::KeyY, "y"),
        (Code::KeyU, "u"),
        (Code::KeyI, "i"),
        (Code::KeyO, "o"),
        (Code::KeyP, "p"),
        (Code::BracketLeft, "["),
        (Code::BracketRight, "]"),
        (Code::Backslash, "\\"),
        (Code::KeyA, "a"),
        (Code::KeyS, "s"),
        (Code::KeyD, "d"),
        (Code::KeyF, "f"),
        (Code::KeyG, "g"),
        (Code::KeyH, "h"),
        (Code::KeyJ, "j"),
        (Code::KeyK, "k"),
        (Code::KeyL, "l"),
        (Code::Semicolon, ";"),
        (Code::Quote, "'"),
        (Code::KeyZ, "z"),
        (Code::KeyX, "x"),
        (Code::KeyC, "c"),
        (Code::KeyV, "v"),
        (Code::KeyB, "b"),
        (Code::KeyN, "n"),
        (Code::KeyM, "m"),
        (Code::Comma, ","),
        (Code::Period, "."),
        (Code::Slash, "/"),
    ];

    /// French AZERTY as Windows and Linux ship it: `-` on the 6 key, `=` right
    /// of `)`.
    const AZERTY_PC: Layout = &[
        (Code::Backquote, "²"),
        (Code::Digit1, "&"),
        (Code::Digit2, "é"),
        (Code::Digit3, "\""),
        (Code::Digit4, "'"),
        (Code::Digit5, "("),
        (Code::Digit6, "-"),
        (Code::Digit7, "è"),
        (Code::Digit8, "_"),
        (Code::Digit9, "ç"),
        (Code::Digit0, "à"),
        (Code::Minus, ")"),
        (Code::Equal, "="),
        (Code::KeyQ, "a"),
        (Code::KeyW, "z"),
        (Code::KeyE, "e"),
        (Code::KeyR, "r"),
        (Code::KeyT, "t"),
        (Code::KeyY, "y"),
        (Code::KeyU, "u"),
        (Code::KeyI, "i"),
        (Code::KeyO, "o"),
        (Code::KeyP, "p"),
        (Code::BracketRight, "$"),
        (Code::Backslash, "*"),
        (Code::KeyA, "q"),
        (Code::KeyS, "s"),
        (Code::KeyD, "d"),
        (Code::KeyF, "f"),
        (Code::KeyG, "g"),
        (Code::KeyH, "h"),
        (Code::KeyJ, "j"),
        (Code::KeyK, "k"),
        (Code::KeyL, "l"),
        (Code::Semicolon, "m"),
        (Code::Quote, "ù"),
        (Code::IntlBackslash, "<"),
        (Code::KeyZ, "w"),
        (Code::KeyX, "x"),
        (Code::KeyC, "c"),
        (Code::KeyV, "v"),
        (Code::KeyB, "b"),
        (Code::KeyN, "n"),
        (Code::KeyM, ","),
        (Code::Comma, ";"),
        (Code::Period, ":"),
        (Code::Slash, "!"),
    ];

    /// French AZERTY as macOS ships it: `-` right of `)`, `=` right of `:`.
    const AZERTY_MAC: Layout = &[
        (Code::Digit1, "&"),
        (Code::Digit2, "é"),
        (Code::Digit3, "\""),
        (Code::Digit4, "'"),
        (Code::Digit5, "("),
        (Code::Digit6, "§"),
        (Code::Digit7, "è"),
        (Code::Digit8, "!"),
        (Code::Digit9, "ç"),
        (Code::Digit0, "à"),
        (Code::Minus, ")"),
        (Code::Equal, "-"),
        (Code::KeyQ, "a"),
        (Code::KeyW, "z"),
        (Code::KeyE, "e"),
        (Code::KeyR, "r"),
        (Code::KeyT, "t"),
        (Code::KeyY, "y"),
        (Code::KeyU, "u"),
        (Code::KeyI, "i"),
        (Code::KeyO, "o"),
        (Code::KeyP, "p"),
        (Code::BracketRight, "$"),
        (Code::KeyA, "q"),
        (Code::KeyS, "s"),
        (Code::KeyD, "d"),
        (Code::KeyF, "f"),
        (Code::KeyG, "g"),
        (Code::KeyH, "h"),
        (Code::KeyJ, "j"),
        (Code::KeyK, "k"),
        (Code::KeyL, "l"),
        (Code::Semicolon, "m"),
        (Code::Quote, "ù"),
        (Code::KeyZ, "w"),
        (Code::KeyX, "x"),
        (Code::KeyC, "c"),
        (Code::KeyV, "v"),
        (Code::KeyB, "b"),
        (Code::KeyN, "n"),
        (Code::KeyM, ","),
        (Code::Comma, ";"),
        (Code::Period, ":"),
        (Code::Slash, "="),
    ];

    const LAYOUTS: [(&str, Layout); 3] = [
        ("QWERTY", QWERTY_US),
        ("AZERTY (PC)", AZERTY_PC),
        ("AZERTY (Mac)", AZERTY_MAC),
    ];

    /// The keypad operators, the same on every layout.
    const KEYPAD: Layout = &[
        (Code::NumpadAdd, "+"),
        (Code::NumpadSubtract, "-"),
        (Code::NumpadMultiply, "*"),
        (Code::NumpadDivide, "/"),
    ];

    /// Whether some press on `layout` reads back as `chord`. A keypad
    /// operator counts only without Shift: a default holding Shift spells a
    /// typing key, and one reachable only by holding Shift over the keypad is
    /// that key's shifted face misnamed.
    fn reachable_on(layout: Layout, chord: &KeyChord) -> bool {
        let modifiers = modifiers_of(chord.mods);
        let press = |&(code, typed): &(Code, &str)| {
            chord_of(
                &Key::Character(typed.into()),
                &Physical::Code(code),
                modifiers,
            )
        };
        let keypad = if chord.mods & keymap::MOD_SHIFT == 0 {
            KEYPAD
        } else {
            &[]
        };
        layout
            .iter()
            .chain(keypad)
            .map(press)
            .chain(
                NAMED_KEYS
                    .iter()
                    .map(|row| chord_of(&Key::Named(row.key), &NON_DIGIT, modifiers)),
            )
            .any(|read| read.as_ref() == Some(chord))
    }

    #[test]
    fn every_default_chord_is_produced_by_a_real_press() {
        // A chord no press produces is a binding that never fires, and the
        // keystroke it was meant for falls through to the terminal.
        for platform in Platform::ALL {
            for (chord, action) in default_bindings(platform) {
                assert!(
                    LAYOUTS
                        .iter()
                        .any(|(_, layout)| reachable_on(layout, &chord)),
                    "default {chord:?} for {action:?} on {platform:?} is produced by no press"
                );
            }
        }
    }

    #[test]
    fn every_default_action_is_reachable_on_qwerty_and_azerty() {
        for platform in Platform::ALL {
            let bindings = default_bindings(platform);
            for (_, action) in &bindings {
                for (name, layout) in LAYOUTS {
                    assert!(
                        bindings
                            .iter()
                            .any(|(chord, bound)| bound == action && reachable_on(layout, chord)),
                        "{action:?} on {platform:?} has no default a {name} press produces"
                    );
                }
            }
        }
    }

    #[test]
    fn zoom_in_is_reachable_from_the_plus_key_with_or_without_shift() {
        use termherd_core::Action;
        // The `+` face of this key needs Shift on all three layouts, so the
        // press reports `=` either way.
        for (layout, physical) in [
            ("QWERTY", Code::Equal),
            ("AZERTY (PC)", Code::Equal),
            ("AZERTY (Mac)", Code::Slash),
        ] {
            for (platform, primary) in [
                (Platform::MacOs, Modifiers::LOGO),
                (Platform::Linux, Modifiers::CTRL),
                (Platform::Windows, Modifiers::CTRL),
            ] {
                for modifiers in [primary, primary | Modifiers::SHIFT] {
                    assert_eq!(
                        default_action_of(platform, "=", physical, modifiers),
                        Some(Action::ZoomIn),
                        "{layout} {modifiers:?} on {platform:?}"
                    );
                }
                assert_eq!(
                    default_action_of(platform, "+", Code::NumpadAdd, primary),
                    Some(Action::ZoomIn),
                    "keypad + on {platform:?}"
                );
            }
        }
    }
}
