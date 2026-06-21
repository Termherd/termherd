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
    /// Jump the focused terminal's viewport to the top of its scrollback (#44).
    ScrollTop,
    /// Jump the focused terminal's viewport back to the live bottom (#44).
    ScrollBottom,
    /// Jump straight to the tab at this zero-based index (issue #26). Bound to
    /// the platform's primary modifier and the number row — ⌘1…⌘9 on macOS,
    /// Ctrl+1…Ctrl+9 elsewhere — where the user-facing digit is 1-based.
    ActivateTab(usize),
}

/// One simple (non-parameterized) action's full definition: the kebab-case name
/// it answers to in the `keys` section of `settings.json`, and its default
/// chord specs. `mod` in a spec resolves to the platform primary modifier
/// (⌘ on macOS, Ctrl elsewhere); other modifiers are literal. An empty
/// `default_chords` means "no built-in binding yet" (wired as the feature lands)
/// or "bound explicitly in [`Keymap::defaults`]" (the platform-irregular pair).
struct ActionDef {
    action: Action,
    name: &'static str,
    default_chords: &'static [&'static str],
}

/// The single source of truth for the simple action vocabulary *and* its
/// defaults (#71): `from_config_name`, `config_name`, [`Keymap::defaults`] and
/// the tests all derive from this one table, so adding a regular action is a
/// single line here. The parameterized [`Action::ActivateTab`] family is named
/// by [`activate_tab_from_config_name`] (its name carries an index); copy/paste
/// carry no spec here because their non-macOS terminal-signal handling is
/// irregular and stays explicit in [`Keymap::defaults`].
const ACTIONS: &[ActionDef] = &[
    ActionDef {
        action: Action::OpenNewSession,
        name: "open-new-session",
        default_chords: &[],
    },
    ActionDef {
        action: Action::CloseFocused,
        name: "close-focused",
        default_chords: &["mod+w"],
    },
    ActionDef {
        action: Action::SplitHorizontal,
        name: "split-horizontal",
        default_chords: &[],
    },
    ActionDef {
        action: Action::SplitVertical,
        name: "split-vertical",
        default_chords: &[],
    },
    ActionDef {
        action: Action::FocusNext,
        name: "focus-next",
        default_chords: &[],
    },
    ActionDef {
        action: Action::FocusPrev,
        name: "focus-prev",
        default_chords: &[],
    },
    ActionDef {
        action: Action::NextTab,
        name: "next-tab",
        default_chords: &["ctrl+tab"],
    },
    ActionDef {
        action: Action::PrevTab,
        name: "prev-tab",
        default_chords: &["ctrl+shift+tab"],
    },
    ActionDef {
        action: Action::FocusSearch,
        name: "focus-search",
        default_chords: &["mod+f"],
    },
    ActionDef {
        action: Action::ToggleSidebar,
        name: "toggle-sidebar",
        default_chords: &["mod+b"],
    },
    ActionDef {
        action: Action::Copy,
        name: "copy",
        default_chords: &[],
    },
    ActionDef {
        action: Action::Paste,
        name: "paste",
        default_chords: &[],
    },
    ActionDef {
        action: Action::ScrollTop,
        name: "scroll-top",
        default_chords: &["mod+up"],
    },
    ActionDef {
        action: Action::ScrollBottom,
        name: "scroll-bottom",
        default_chords: &["mod+down"],
    },
];

impl Action {
    /// The action for a config key name (kebab-case), or `None` if unknown.
    /// This is the vocabulary the `keys` section of `settings.json` speaks.
    #[must_use]
    pub fn from_config_name(name: &str) -> Option<Action> {
        ACTIONS
            .iter()
            .find(|def| def.name == name)
            .map(|def| def.action)
            .or_else(|| activate_tab_from_config_name(name))
    }

    /// The config name for this action, or `None` for the parameterized
    /// [`Action::ActivateTab`] (named `activate-tab-N`, not in [`ACTIONS`]).
    #[must_use]
    pub fn config_name(self) -> Option<&'static str> {
        ACTIONS
            .iter()
            .find(|def| def.action == self)
            .map(|def| def.name)
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

/// Parse a default chord spec from [`ACTIONS`], resolving the `mod` token to the
/// platform primary modifier so one spec serves both ⌘ and Ctrl. Returns `None`
/// on a malformed spec; specs are authored in-tree and a test asserts they all
/// parse, so `None` never reaches a real keymap (and `core` forbids panicking).
fn default_chord(spec: &str) -> Option<KeyChord> {
    let primary = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    KeyChord::parse(&spec.replace("mod", primary)).ok()
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
    /// The built-in bindings. Regular actions come straight from the [`ACTIONS`]
    /// table's default chords. Copy/paste are the exception — the other
    /// platforms keep plain Ctrl+C/V as the terminal interrupt/literal and use
    /// Ctrl+Shift+C for copy — so they are bound explicitly here.
    pub fn defaults() -> Self {
        let mut map = Keymap {
            bindings: HashMap::new(),
        };
        // Regular actions: their default chords are data in the table.
        for def in ACTIONS {
            let chords: Vec<KeyChord> = def
                .default_chords
                .iter()
                .copied()
                .filter_map(default_chord)
                .collect();
            if !chords.is_empty() {
                map.set(def.action, chords);
            }
        }
        // Copy/paste are platform-irregular: see the note above.
        if cfg!(target_os = "macos") {
            map.set(Action::Copy, [KeyChord::new("c", MOD_CMD)]);
            map.set(Action::Paste, [KeyChord::new("v", MOD_CMD)]);
        } else {
            map.set(Action::Copy, [KeyChord::new("c", MOD_CTRL | MOD_SHIFT)]);
            map.set(
                Action::Paste,
                [
                    KeyChord::new("v", MOD_CTRL),
                    KeyChord::new("v", MOD_CTRL | MOD_SHIFT),
                ],
            );
        }
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
    fn the_action_vocabulary_round_trips_both_ways() {
        // Every entry in the single-source table resolves by name and back to
        // the same name, so name↔action can never drift (#71).
        for def in ACTIONS {
            assert_eq!(
                Action::from_config_name(def.name),
                Some(def.action),
                "config action `{}` should map to {:?}",
                def.name,
                def.action,
            );
            assert_eq!(
                def.action.config_name(),
                Some(def.name),
                "{:?} should report its config name `{}`",
                def.action,
                def.name,
            );
        }
    }

    #[test]
    fn every_default_chord_spec_in_the_table_parses() {
        // The specs are authored in-tree; a typo would silently drop a default
        // binding (defaults() skips unparsable specs to keep core panic-free).
        // Fail here instead so it never ships.
        for def in ACTIONS {
            for spec in def.default_chords {
                assert!(
                    default_chord(spec).is_some(),
                    "default chord spec `{spec}` for {:?} must parse",
                    def.action
                );
            }
        }
    }

    #[test]
    fn every_default_bound_action_is_reconfigurable() {
        // Guards the link between `defaults` and the vocabulary: a default-bound
        // action with no config name would be unrebindable. ActivateTab is named
        // separately (`activate-tab-N`), so it is allowed without a table entry.
        let map = Keymap::defaults();
        for action in map.bindings.values() {
            let named = action.config_name().is_some() || matches!(action, Action::ActivateTab(_));
            assert!(named, "default-bound {action:?} has no config name (#71)");
        }
    }

    #[test]
    fn defaults_bind_scroll_top_and_bottom_to_the_primary_modifier_arrows() {
        let map = Keymap::defaults();
        assert_eq!(
            map.lookup(&KeyChord::new("up", primary_mod())),
            Some(Action::ScrollTop)
        );
        assert_eq!(
            map.lookup(&KeyChord::new("down", primary_mod())),
            Some(Action::ScrollBottom)
        );
        assert_eq!(
            Action::from_config_name("scroll-top"),
            Some(Action::ScrollTop)
        );
        assert_eq!(
            Action::from_config_name("scroll-bottom"),
            Some(Action::ScrollBottom)
        );
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
