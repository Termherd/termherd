//! Keyboard translation: iced key events → the framework-neutral types the
//! rest of the app speaks. Two targets — keymap chords (FR9 shortcuts) and the
//! PTY codec's [`TermKey`]/[`KeyMods`] — plus the held-modifier helper for the
//! link-open gesture. Pure functions, no `Shell` coupling.

use iced::keyboard;
use termherd_core::{KeyChord, keymap};
use termherd_pty::{KeyMods, TermKey};

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

/// The keymap name of an iced key: a lowercased character, or a handful of
/// named keys that bindings use. `None` for keys no shortcut can target.
fn key_name(key: &keyboard::Key) -> Option<String> {
    use keyboard::key::Named;
    match key {
        keyboard::Key::Character(c) => c
            .chars()
            .next()
            .map(|ch| ch.to_ascii_lowercase().to_string()),
        keyboard::Key::Named(Named::Tab) => Some("tab".to_string()),
        keyboard::Key::Named(Named::Enter) => Some("enter".to_string()),
        keyboard::Key::Named(Named::Escape) => Some("escape".to_string()),
        // Arrow keys carry chord names so bindings like `mod+up` (scroll) and
        // `mod+shift+left` (pane focus) resolve — without this they fall through
        // to the terminal, leaking the raw cursor sequence to the shell.
        keyboard::Key::Named(Named::ArrowUp) => Some("up".to_string()),
        keyboard::Key::Named(Named::ArrowDown) => Some("down".to_string()),
        keyboard::Key::Named(Named::ArrowLeft) => Some("left".to_string()),
        keyboard::Key::Named(Named::ArrowRight) => Some("right".to_string()),
        _ => None,
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
    use keyboard::Key;
    use keyboard::key::Named;
    match key {
        Key::Character(c) => c.chars().next().map(TermKey::Char),
        Key::Named(Named::Enter) => Some(TermKey::Enter),
        Key::Named(Named::Tab) => Some(TermKey::Tab),
        Key::Named(Named::Backspace) => Some(TermKey::Backspace),
        Key::Named(Named::Escape) => Some(TermKey::Escape),
        Key::Named(Named::Space) => Some(TermKey::Space),
        Key::Named(Named::ArrowUp) => Some(TermKey::Up),
        Key::Named(Named::ArrowDown) => Some(TermKey::Down),
        Key::Named(Named::ArrowLeft) => Some(TermKey::Left),
        Key::Named(Named::ArrowRight) => Some(TermKey::Right),
        Key::Named(Named::Home) => Some(TermKey::Home),
        Key::Named(Named::End) => Some(TermKey::End),
        Key::Named(Named::Delete) => Some(TermKey::Delete),
        Key::Named(Named::PageUp) => Some(TermKey::PageUp),
        Key::Named(Named::PageDown) => Some(TermKey::PageDown),
        _ => None,
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
    use super::*;
    use iced::keyboard::key::{Code, Named, NativeCode, Physical};
    use iced::keyboard::{Key, Modifiers};

    /// A physical key that is not on the number row — letters and named keys
    /// fall back to their character/name, so tests pass this for non-digits.
    const NON_DIGIT: Physical = Physical::Unidentified(NativeCode::Unidentified);

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
        // Keys with no terminal sequence reach no PTY.
        assert_eq!(to_term_key(&Key::Named(Named::F2)), None);
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
        // Keys no shortcut targets carry no chord.
        assert_eq!(
            chord_of(&Key::Named(Named::F2), &NON_DIGIT, Modifiers::default()),
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
}
