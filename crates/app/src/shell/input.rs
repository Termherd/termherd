//! Keyboard translation: iced key events → the framework-neutral types the
//! rest of the app speaks. Two targets — keymap chords (FR9 shortcuts) and the
//! PTY codec's [`TermKey`]/[`KeyMods`] — plus the held-modifier helper for the
//! link-open gesture (#28). Pure functions, no `Shell` coupling.

use iced::keyboard;
use termherd_core::{KeyChord, keymap};
use termherd_pty::{KeyMods, TermKey};

/// The keymap chord for a key press (FR9): the key's normalised name plus the
/// modifier bits. `None` for keys we do not bind (so they reach the terminal).
pub(super) fn chord_of(key: &keyboard::Key, modifiers: keyboard::Modifiers) -> Option<KeyChord> {
    let name = key_name(key)?;
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
        _ => None,
    }
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
/// keep the link-open modifier tracked across press / release / change (#28).
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
    use iced::keyboard::key::Named;
    use iced::keyboard::{Key, Modifiers};

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
            chord_of(&Key::Character("C".into()), ctrl_shift),
            Some(KeyChord::new("c", keymap::MOD_CTRL | keymap::MOD_SHIFT))
        );
        assert_eq!(
            chord_of(&Key::Named(Named::Tab), Modifiers::CTRL),
            Some(KeyChord::new("tab", keymap::MOD_CTRL))
        );
        // Keys no shortcut targets carry no chord.
        assert_eq!(chord_of(&Key::Named(Named::F2), Modifiers::default()), None);
    }
}
