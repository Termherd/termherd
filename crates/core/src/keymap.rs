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

/// Tabs reachable from the number row: ⌘1…⌘9 / Ctrl+1…Ctrl+9 (issue #26). The
/// digit is 1-based for the user; the tab index it carries is 0-based.
pub const NUMBER_ROW_TABS: usize = 9;

/// The platform's primary command modifier — ⌘ on macOS, Ctrl elsewhere. This
/// is the modifier the number-row tab jumps bind to by default.
#[must_use]
pub const fn primary_mod() -> u8 {
    if cfg!(target_os = "macos") {
        MOD_CMD
    } else {
        MOD_CTRL
    }
}

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
    ToggleSidebar,
    Copy,
    Paste,
    /// Jump straight to the tab at this zero-based index (issue #26). Bound to
    /// the platform's primary modifier and the number row — ⌘1…⌘9 on macOS,
    /// Ctrl+1…Ctrl+9 elsewhere — where the user-facing digit is 1-based.
    ActivateTab(usize),
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
            "toggle-sidebar" => Action::ToggleSidebar,
            "copy" => Action::Copy,
            "paste" => Action::Paste,
            _ => return activate_tab_from_config_name(name),
        })
    }
}

/// Parse an `activate-tab-N` config name (N in `1..=NUMBER_ROW_TABS`) into the
/// matching zero-based [`Action::ActivateTab`], or `None` for anything else.
fn activate_tab_from_config_name(name: &str) -> Option<Action> {
    let digit = name.strip_prefix("activate-tab-")?;
    let n: usize = digit.parse().ok()?;
    // Lazy `then` (not `then_some`): `n - 1` must not be evaluated for n = 0,
    // where it would underflow `usize` before the range check rejects it.
    (1..=NUMBER_ROW_TABS)
        .contains(&n)
        .then(|| Action::ActivateTab(n - 1))
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
            map.set(Action::ToggleSidebar, [KeyChord::new("b", MOD_CMD)]);
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
            map.set(Action::ToggleSidebar, [KeyChord::new("b", MOD_CTRL)]);
        }
        map.set(Action::NextTab, [KeyChord::new("tab", MOD_CTRL)]);
        map.set(
            Action::PrevTab,
            [KeyChord::new("tab", MOD_CTRL | MOD_SHIFT)],
        );
        // Jump straight to the Nth tab: ⌘1…⌘9 / Ctrl+1…Ctrl+9 (issue #26). The
        // digit is 1-based for the user; the action carries the 0-based index.
        for n in 1..=NUMBER_ROW_TABS {
            map.set(
                Action::ActivateTab(n - 1),
                [KeyChord::new(n.to_string(), primary_mod())],
            );
        }
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
    fn toggle_sidebar_binds_to_the_platform_b_chord() {
        let map = Keymap::defaults();
        let mods = if cfg!(target_os = "macos") {
            MOD_CMD
        } else {
            MOD_CTRL
        };
        assert_eq!(
            map.lookup(&KeyChord::new("b", mods)),
            Some(Action::ToggleSidebar)
        );
        assert_eq!(
            Action::from_config_name("toggle-sidebar"),
            Some(Action::ToggleSidebar)
        );
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
    fn primary_mod_is_the_platform_command_modifier() {
        // Pinned to the concrete constant (not derived from `primary_mod`) so a
        // regression that swaps ⌘ for Ctrl on macOS is actually caught.
        if cfg!(target_os = "macos") {
            assert_eq!(primary_mod(), MOD_CMD);
        } else {
            assert_eq!(primary_mod(), MOD_CTRL);
        }
    }

    #[test]
    fn every_config_action_name_maps_to_an_action() {
        // The full `keys` vocabulary the README documents must stay resolvable.
        for name in [
            "open-new-session",
            "close-focused",
            "split-horizontal",
            "split-vertical",
            "focus-next",
            "focus-prev",
            "next-tab",
            "prev-tab",
            "focus-search",
            "toggle-sidebar",
            "copy",
            "paste",
        ] {
            assert!(
                Action::from_config_name(name).is_some(),
                "config action `{name}` should map to an Action",
            );
        }
    }

    #[test]
    fn defaults_bind_the_number_row_to_tab_jumps() {
        let map = Keymap::defaults();
        // 1-based digit, 0-based tab index.
        assert_eq!(
            map.lookup(&KeyChord::new("1", primary_mod())),
            Some(Action::ActivateTab(0))
        );
        assert_eq!(
            map.lookup(&KeyChord::new("9", primary_mod())),
            Some(Action::ActivateTab(8))
        );
        // The row stops at nine: there is no zero or tenth binding.
        assert_eq!(map.lookup(&KeyChord::new("0", primary_mod())), None);
        // A bare digit without the modifier is left for the terminal.
        assert_eq!(map.lookup(&KeyChord::new("1", 0)), None);
    }

    #[test]
    fn activate_tab_config_names_reject_out_of_range_and_garbage() {
        assert_eq!(Action::from_config_name("activate-tab-0"), None);
        assert_eq!(Action::from_config_name("activate-tab-10"), None);
        assert_eq!(Action::from_config_name("activate-tab-"), None);
        assert_eq!(Action::from_config_name("activate-tab-x"), None);
        assert_eq!(Action::from_config_name("activate-tab"), None);
    }

    proptest::proptest! {
        /// Every digit on the number row resolves to its zero-based tab jump.
        #[test]
        fn every_number_row_digit_binds_to_its_zero_based_tab(n in 1usize..=9) {
            let map = Keymap::defaults();
            proptest::prop_assert_eq!(
                map.lookup(&KeyChord::new(n.to_string(), primary_mod())),
                Some(Action::ActivateTab(n - 1))
            );
        }

        /// `activate-tab-N` config names round-trip to the matching jump.
        #[test]
        fn activate_tab_config_names_round_trip(n in 1usize..=9) {
            proptest::prop_assert_eq!(
                Action::from_config_name(&format!("activate-tab-{n}")),
                Some(Action::ActivateTab(n - 1))
            );
        }

        /// A digit past the number row is not a known config action.
        #[test]
        fn activate_tab_names_past_nine_are_unknown(n in 10usize..10_000) {
            proptest::prop_assert_eq!(
                Action::from_config_name(&format!("activate-tab-{n}")),
                None
            );
        }
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
