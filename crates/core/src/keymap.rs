//! Keymap — `KeyChord -> Action`.
//!
//! Pure and configurable (FR9, OQ2): the shell builds a [`KeyChord`] from a raw
//! key event and asks the [`Keymap`] for the [`Action`] to run. Default
//! bindings are platform-aware (macOS uses ⌘); the app overrides them from
//! `~/.termherd/settings.json`. No I/O here — parsing and lookup are pure, so
//! every binding decision is unit-testable headless.

use std::collections::HashMap;

/// Modifier bit for the Ctrl key.
pub const MOD_CTRL: u8 = 1;
/// Modifier bit for the Alt / Option key.
pub const MOD_ALT: u8 = 2;
/// Modifier bit for the Shift key.
pub const MOD_SHIFT: u8 = 4;
/// Modifier bit for the Cmd / Super / logo key.
pub const MOD_CMD: u8 = 8;

/// A key plus its modifiers — the left-hand side of a binding. `key` is a
/// normalised lowercase name (`"c"`, `"tab"`, `"enter"`); `mods` is the OR of
/// the `MOD_*` bits.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub key: String,
    pub mods: u8,
}

/// Why a chord string failed to parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChordError {
    /// The string named modifiers but no key (e.g. `"ctrl+"`).
    #[error("chord has no key")]
    NoKey,
    /// The string named more than one non-modifier key (e.g. `"a+b"`).
    #[error("chord has more than one key")]
    MultipleKeys,
}

impl KeyChord {
    /// A chord from a key name and modifier bits. The key is lowercased so
    /// lookups are case-insensitive.
    pub fn new(key: impl Into<String>, mods: u8) -> Self {
        Self {
            key: key.into().to_ascii_lowercase(),
            mods,
        }
    }

    /// Parse a human chord like `"ctrl+shift+c"` or `"cmd+tab"`. Order does not
    /// matter and the parse is case-insensitive. Modifier aliases: `control`,
    /// `option` (Alt), `cmd`/`super`/`win`/`meta` (Cmd).
    pub fn parse(s: &str) -> Result<KeyChord, ChordError> {
        let mut mods = 0u8;
        let mut key: Option<String> = None;
        for part in s.split('+') {
            let token = part.trim().to_ascii_lowercase();
            if token.is_empty() {
                continue;
            }
            match token.as_str() {
                "ctrl" | "control" => mods |= MOD_CTRL,
                "alt" | "option" => mods |= MOD_ALT,
                "shift" => mods |= MOD_SHIFT,
                "cmd" | "super" | "logo" | "win" | "meta" => mods |= MOD_CMD,
                _ => {
                    if key.is_some() {
                        return Err(ChordError::MultipleKeys);
                    }
                    key = Some(token);
                }
            }
        }
        match key {
            Some(key) => Ok(KeyChord { key, mods }),
            None => Err(ChordError::NoKey),
        }
    }
}

/// A configurable workspace command (FR9). Variants without a default binding
/// yet (splits, focus moves) are wired as their features land.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    OpenNewSession,
    CloseFocused,
    SplitHorizontal,
    SplitVertical,
    FocusNext,
    FocusPrev,
    NextTab,
    PrevTab,
    FocusSearch,
    Copy,
    Paste,
}

impl Action {
    /// The action for a config key name (kebab-case), or `None` if unknown.
    /// This is the vocabulary the `keys` section of `settings.json` speaks.
    #[must_use]
    pub fn from_config_name(name: &str) -> Option<Action> {
        Some(match name {
            "open-new-session" => Action::OpenNewSession,
            "close-focused" => Action::CloseFocused,
            "split-horizontal" => Action::SplitHorizontal,
            "split-vertical" => Action::SplitVertical,
            "focus-next" => Action::FocusNext,
            "focus-prev" => Action::FocusPrev,
            "next-tab" => Action::NextTab,
            "prev-tab" => Action::PrevTab,
            "focus-search" => Action::FocusSearch,
            "copy" => Action::Copy,
            "paste" => Action::Paste,
            _ => return None,
        })
    }
}

/// Resolves a [`KeyChord`] to its [`Action`]. Built from platform-aware
/// defaults, then overridden per the user's config.
#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: HashMap<KeyChord, Action>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::defaults()
    }
}

impl Keymap {
    /// The built-in bindings. macOS binds copy/paste/close/search to ⌘; the
    /// other platforms use Ctrl (and Ctrl+Shift+C for copy, so plain Ctrl+C
    /// stays the interrupt signal). Tab cycling is the same everywhere.
    pub fn defaults() -> Self {
        let mut map = Keymap {
            bindings: HashMap::new(),
        };
        if cfg!(target_os = "macos") {
            map.set(Action::Copy, [KeyChord::new("c", MOD_CMD)]);
            map.set(Action::Paste, [KeyChord::new("v", MOD_CMD)]);
            map.set(Action::CloseFocused, [KeyChord::new("w", MOD_CMD)]);
            map.set(Action::FocusSearch, [KeyChord::new("f", MOD_CMD)]);
        } else {
            map.set(Action::Copy, [KeyChord::new("c", MOD_CTRL | MOD_SHIFT)]);
            map.set(
                Action::Paste,
                [
                    KeyChord::new("v", MOD_CTRL),
                    KeyChord::new("v", MOD_CTRL | MOD_SHIFT),
                ],
            );
            map.set(Action::CloseFocused, [KeyChord::new("w", MOD_CTRL)]);
            map.set(Action::FocusSearch, [KeyChord::new("f", MOD_CTRL)]);
        }
        map.set(Action::NextTab, [KeyChord::new("tab", MOD_CTRL)]);
        map.set(
            Action::PrevTab,
            [KeyChord::new("tab", MOD_CTRL | MOD_SHIFT)],
        );
        map
    }

    /// Bind `action` to exactly `chords`, dropping any chords previously bound
    /// to it. This is how a user override replaces a default.
    pub fn set(&mut self, action: Action, chords: impl IntoIterator<Item = KeyChord>) {
        self.bindings.retain(|_, bound| *bound != action);
        for chord in chords {
            self.bindings.insert(chord, action);
        }
    }

    /// The action bound to `chord`, if any.
    #[must_use]
    pub fn lookup(&self, chord: &KeyChord) -> Option<Action> {
        self.bindings.get(chord).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_modifiers_and_key_in_any_order() {
        assert_eq!(
            KeyChord::parse("ctrl+shift+c"),
            Ok(KeyChord::new("c", MOD_CTRL | MOD_SHIFT))
        );
        // Order and case are irrelevant.
        assert_eq!(
            KeyChord::parse("C+SHIFT+CTRL"),
            Ok(KeyChord::new("c", MOD_CTRL | MOD_SHIFT))
        );
        assert_eq!(
            KeyChord::parse("cmd+tab"),
            Ok(KeyChord::new("tab", MOD_CMD))
        );
    }

    #[test]
    fn parse_rejects_missing_or_doubled_keys() {
        assert_eq!(KeyChord::parse("ctrl+"), Err(ChordError::NoKey));
        assert_eq!(KeyChord::parse("ctrl+shift"), Err(ChordError::NoKey));
        assert_eq!(KeyChord::parse("a+b"), Err(ChordError::MultipleKeys));
    }

    #[test]
    fn defaults_resolve_copy_and_paste_for_the_platform() {
        let map = Keymap::defaults();
        if cfg!(target_os = "macos") {
            assert_eq!(map.lookup(&KeyChord::new("c", MOD_CMD)), Some(Action::Copy));
            assert_eq!(
                map.lookup(&KeyChord::new("v", MOD_CMD)),
                Some(Action::Paste)
            );
        } else {
            assert_eq!(
                map.lookup(&KeyChord::new("c", MOD_CTRL | MOD_SHIFT)),
                Some(Action::Copy)
            );
            assert_eq!(
                map.lookup(&KeyChord::new("v", MOD_CTRL)),
                Some(Action::Paste)
            );
        }
        // Tab cycling is bound on every platform.
        assert_eq!(
            map.lookup(&KeyChord::new("tab", MOD_CTRL)),
            Some(Action::NextTab)
        );
        // An unbound chord resolves to nothing.
        assert_eq!(map.lookup(&KeyChord::new("q", MOD_CTRL)), None);
    }

    #[test]
    fn config_names_round_trip_to_actions() {
        assert_eq!(Action::from_config_name("copy"), Some(Action::Copy));
        assert_eq!(Action::from_config_name("next-tab"), Some(Action::NextTab));
        assert_eq!(
            Action::from_config_name("close-focused"),
            Some(Action::CloseFocused)
        );
        assert_eq!(Action::from_config_name("nope"), None);
    }

    #[test]
    fn set_replaces_every_chord_previously_bound_to_an_action() {
        let mut map = Keymap::defaults();
        // Rebind paste to a single new chord; the old paste chords stop working.
        map.set(Action::Paste, [KeyChord::new("v", MOD_CMD)]);
        assert_eq!(
            map.lookup(&KeyChord::new("v", MOD_CMD)),
            Some(Action::Paste)
        );
        assert_eq!(map.lookup(&KeyChord::new("v", MOD_CTRL)), None);
    }
}
