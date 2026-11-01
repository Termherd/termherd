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

/// Tabs reachable from the number row: ⌘1…⌘9 / Ctrl+1…Ctrl+9. The
/// digit is 1-based for the user; the tab index it carries is 0-based.
pub const NUMBER_ROW_TABS: usize = 9;

/// The platform's primary command modifier — ⌘ on macOS, Ctrl elsewhere. This
/// is the modifier the number-row tab jumps bind to by default.
#[must_use]
pub const fn primary_mod() -> u8 {
    Platform::current().primary_mod()
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
    /// `option` (Alt), `cmd`/`super`/`win`/`meta` (Cmd). A literal `+` key
    /// cannot survive the `+` separator split, so it is spelled `plus`.
    pub fn parse(s: &str) -> Result<KeyChord, ChordError> {
        let mut mods = 0u8;
        let mut key: Option<String> = None;
        for part in s.split('+') {
            let token = part.trim().to_ascii_lowercase();
            if token.is_empty() {
                continue;
            }
            let token = if token == "plus" {
                "+".to_owned()
            } else {
                token
            };
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
    /// Move pane focus one step in a spatial direction, cycling within the
    /// move's axis (FR6). Left/Right traverse vertical splits, Up/Down
    /// horizontal ones.
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    NextTab,
    PrevTab,
    FocusSearch,
    ToggleSidebar,
    /// Open the settings panel, or close it when it is already open.
    OpenSettings,
    Copy,
    Paste,
    /// Copy the focused Claude's peer name, the one other Claude sessions
    /// address it by. Inert when no Claude in front of it has written one.
    CopyAgentName,
    /// Open the inline rename of the focused tab, seeded with its shown title.
    /// Inert when no tab is open.
    RenameTab,
    /// Open the focused tab's context menu, whose entries are keymap actions.
    /// Inert when no tab is open.
    OpenTabMenu,
    /// Ask to hand the focused Claude session to the Claude desktop app by
    /// typing `/desktop` — behind the same confirmation as every command
    /// termherd types into Claude. Inert unless the focused pane is a Claude
    /// idle at its prompt.
    SendToDesktop,
    /// Open the focused tab's colour picker. A pick for a Claude pane is typed
    /// as `/color`, behind the same confirmation; a shell tab keeps it itself.
    /// Inert with no tab, or on a Claude pane that cannot take a command now.
    PickTabColor,
    /// Jump the focused terminal's viewport to the top of its scrollback.
    ScrollTop,
    /// Jump the focused terminal's viewport back to the live bottom.
    ScrollBottom,
    /// Open a fresh shell in the focused session's working directory, or the
    /// home directory when nothing is open.
    NewShellHere,
    /// Open a fresh Claude session in the repo containing the focused session.
    /// Inert when there is no focused session to derive a repo from.
    NewClaudeSessionHere,
    /// Reopen the most recently closed tab, restoring its mode and directory.
    /// A no-op when no tab has been closed yet.
    ReopenClosedTab,
    /// Capture the current state for the AI dev loop: a JSON state dump
    /// plus a PNG screenshot, written to `~/.termherd/captures/`.
    Capture,
    /// Start or stop the GIF screencast: a toggle that records the window
    /// to `~/.termherd/captures/capture-<ts>.gif`.
    ToggleRecord,
    /// Grow the terminal font one step.
    ZoomIn,
    /// Shrink the terminal font one step.
    ZoomOut,
    /// Restore the terminal font to its configured base size.
    ZoomReset,
    /// Jump straight to the tab at this zero-based index. Bound to
    /// the platform's primary modifier and the number row — ⌘1…⌘9 on macOS,
    /// Ctrl+1…Ctrl+9 elsewhere — where the user-facing digit is 1-based.
    ActivateTab(usize),
}

/// One simple (non-parameterized) action's full definition: the kebab-case name
/// it answers to in the `keys` section of `settings.json`, and its default
/// chord specs per platform. `mod` is ⌘ on macOS, Ctrl elsewhere. No chords
/// means no built-in binding yet.
///
/// `off_mac` replaces `chords` on Windows and Linux; `windows` replaces both on
/// Windows. Off macOS a default must not take a bare Ctrl+letter, which is a
/// terminal control key: those actions take Ctrl+Shift+letter, which a legacy
/// program reads as the same byte.
struct ActionDef {
    action: Action,
    name: &'static str,
    chords: &'static [&'static str],
    off_mac: Option<&'static [&'static str]>,
    windows: Option<&'static [&'static str]>,
}

impl ActionDef {
    /// An action bound to `chords` on every platform.
    const fn new(action: Action, name: &'static str, chords: &'static [&'static str]) -> Self {
        Self {
            action,
            name,
            chords,
            off_mac: None,
            windows: None,
        }
    }

    /// The same action bound to `chords` instead on Windows and Linux.
    const fn off_mac(self, chords: &'static [&'static str]) -> Self {
        Self {
            off_mac: Some(chords),
            ..self
        }
    }

    /// The same action bound to `chords` instead on Windows.
    const fn windows(self, chords: &'static [&'static str]) -> Self {
        Self {
            windows: Some(chords),
            ..self
        }
    }

    /// The default chord specs this action carries on `platform`.
    fn chords_for(&self, platform: Platform) -> &'static [&'static str] {
        let off_mac = self.off_mac.unwrap_or(self.chords);
        match platform {
            Platform::MacOs => self.chords,
            Platform::Windows => self.windows.unwrap_or(off_mac),
            Platform::Linux => off_mac,
        }
    }
}

/// The single source of truth for the simple action vocabulary *and* its
/// defaults: `from_config_name`, `config_name`, [`Keymap::defaults`] and
/// the tests all derive from this one table, so adding a regular action is a
/// single line here. The parameterized [`Action::ActivateTab`] family is named
/// by [`activate_tab_from_config_name`] (its name carries an index).
const ACTIONS: &[ActionDef] = &[
    ActionDef::new(Action::OpenNewSession, "open-new-session", &[]),
    ActionDef::new(Action::CloseFocused, "close-focused", &["mod+w"]).off_mac(&["ctrl+shift+w"]),
    // Off macOS, Windows Terminal's Alt+Shift+Plus / Minus. A chord names its
    // key before Shift, so `+` is `=` on QWERTY and AZERTY. AZERTY puts `-` on
    // the 6 key, named by position, so stacking also takes Alt+Shift+D.
    ActionDef::new(
        Action::SplitHorizontal,
        "split-horizontal",
        &["mod+shift+d"],
    )
    .off_mac(&["alt+shift+-", "alt+shift+d"]),
    ActionDef::new(Action::SplitVertical, "split-vertical", &["mod+d"])
        .off_mac(&["alt+shift+=", "alt+shift+plus"]),
    ActionDef::new(Action::FocusNext, "focus-next", &[]),
    ActionDef::new(Action::FocusPrev, "focus-prev", &[]),
    ActionDef::new(Action::FocusLeft, "focus-left", &["mod+shift+left"]),
    ActionDef::new(Action::FocusRight, "focus-right", &["mod+shift+right"]),
    ActionDef::new(Action::FocusUp, "focus-up", &["mod+shift+up"]),
    ActionDef::new(Action::FocusDown, "focus-down", &["mod+shift+down"]),
    ActionDef::new(Action::NextTab, "next-tab", &["ctrl+tab"]),
    ActionDef::new(Action::PrevTab, "prev-tab", &["ctrl+shift+tab"]),
    ActionDef::new(Action::FocusSearch, "focus-search", &["mod+f"]).off_mac(&["ctrl+shift+f"]),
    ActionDef::new(Action::ToggleSidebar, "toggle-sidebar", &["mod+b"]).off_mac(&["ctrl+shift+b"]),
    ActionDef::new(Action::OpenSettings, "open-settings", &["mod+,"]),
    // Ctrl+C stays the interrupt. Bare Ctrl+V pastes on Windows only, as in
    // Windows Terminal (Claude Code pastes images on Alt+V there); on Linux
    // Claude Code pastes images on it.
    ActionDef::new(Action::Copy, "copy", &["mod+c"]).off_mac(&["ctrl+shift+c"]),
    ActionDef::new(Action::Paste, "paste", &["mod+v"])
        .off_mac(&["ctrl+shift+v"])
        .windows(&["ctrl+shift+v", "ctrl+v"]),
    ActionDef::new(Action::CopyAgentName, "copy-agent-name", &[]),
    ActionDef::new(Action::SendToDesktop, "send-to-desktop", &[]),
    ActionDef::new(Action::PickTabColor, "pick-tab-color", &[]),
    // Ctrl+Up / Down stay the program's; Windows Terminal's Home / End instead.
    ActionDef::new(Action::ScrollTop, "scroll-top", &["mod+up"]).off_mac(&["ctrl+shift+home"]),
    ActionDef::new(Action::ScrollBottom, "scroll-bottom", &["mod+down"])
        .off_mac(&["ctrl+shift+end"]),
    // Ctrl+Shift+T is every Windows/Linux terminal's new tab; reopen moves up.
    ActionDef::new(Action::NewShellHere, "new-shell-here", &["mod+t"]).off_mac(&["ctrl+shift+t"]),
    // Terminal.app binds its "Edit Title" to ⌘⇧I. Elsewhere Ctrl+Shift+I
    // reaches a legacy-encoded program as the same byte as Ctrl+I (Tab), so
    // claiming it takes no key away from anything running in a pane.
    ActionDef::new(Action::RenameTab, "rename-tab", &["mod+shift+i"]),
    // A letter, not punctuation: a press names its key without Shift, so a
    // character that takes Shift on some layout (`.` on AZERTY) is unreachable
    // there. Ctrl+Shift+M is Ctrl+M (Enter) to a legacy terminal, which still
    // reaches a program in the pane unshifted, as with `rename-tab`.
    ActionDef::new(Action::OpenTabMenu, "open-tab-menu", &["mod+shift+m"]),
    ActionDef::new(
        Action::NewClaudeSessionHere,
        "new-claude-session-here",
        &["mod+alt+t"],
    ),
    ActionDef::new(
        Action::ReopenClosedTab,
        "reopen-closed-tab",
        &["mod+shift+t"],
    )
    .off_mac(&["ctrl+alt+shift+t"]),
    ActionDef::new(Action::Capture, "capture", &["mod+shift+s"]),
    ActionDef::new(Action::ToggleRecord, "toggle-record", &["mod+shift+r"]),
    // Zoom: `=` is the unshifted face of the `+` key on QWERTY and the
    // unshifted key on AZERTY; the shifted `plus` spellings cover layouts
    // where the produced character is `+` (with or without Shift reported).
    ActionDef::new(
        Action::ZoomIn,
        "zoom-in",
        &["mod+=", "mod+plus", "mod+shift+plus"],
    ),
    ActionDef::new(Action::ZoomOut, "zoom-out", &["mod+-"]),
    ActionDef::new(Action::ZoomReset, "zoom-reset", &["mod+0"]),
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

    /// This action's config name, **including** the parameterized
    /// [`Action::ActivateTab`] family that [`Self::config_name`] cannot spell
    /// because its name carries an index. Total where `config_name` is partial,
    /// so it round-trips through [`Self::from_config_name`] for every action —
    /// which is what a surface reporting *which* action ran needs.
    #[must_use]
    pub fn name(self) -> String {
        if let Some(name) = self.config_name() {
            return name.to_owned();
        }
        match self {
            // The action carries the 0-based index; the name carries the
            // 1-based digit the user sees, as `from_config_name` reads it.
            Action::ActivateTab(index) => format!("activate-tab-{}", index + 1),
            // Every other variant has a row in `ACTIONS`, so `config_name`
            // already answered. `every_action_round_trips_through_its_name` is
            // what holds that — not this comment. The arm is the type-level
            // residue of a fact a test pins, since `core` forbids the panic that
            // would state it directly; `Debug` at least names the offender if a
            // new variant ever lands without its table row.
            other => format!("{other:?}"),
        }
    }
}

/// One entry of the public, read-only action vocabulary: a bindable action's
/// kebab-case config name and its default chords on one platform, in the
/// syntax `settings.json` accepts. This is the shape an out-of-crate surface
/// enumerates; it does not expose the internal [`Action`] value.
pub struct ActionBinding {
    /// The kebab-case name the `keys` section of `settings.json` speaks.
    pub name: &'static str,
    /// Default chords for this action; empty means "no built-in binding".
    pub default_chords: Vec<String>,
}

/// The bindable simple-action vocabulary with `platform`'s defaults, in table
/// order, derived from [`ACTIONS`]. `activate-tab-N` is not included.
#[must_use]
pub fn action_catalog(platform: Platform) -> Vec<ActionBinding> {
    ACTIONS
        .iter()
        .map(|def| ActionBinding {
            name: def.name,
            default_chords: def
                .chords_for(platform)
                .iter()
                .map(|spec| resolve_spec(spec, platform))
                .collect(),
        })
        .collect()
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

/// A default chord spec from [`ACTIONS`] with its `mod` token spelled as the
/// platform primary modifier, so one spec serves both ⌘ and Ctrl.
fn resolve_spec(spec: &str, platform: Platform) -> String {
    spec.replace("mod", platform.primary_name())
}

/// Parse a default chord spec from [`ACTIONS`]. Returns `None` on a malformed
/// spec; specs are authored in-tree and a test asserts they all parse, so
/// `None` never reaches a real keymap (and `core` forbids panicking).
fn default_chord(spec: &str, platform: Platform) -> Option<KeyChord> {
    KeyChord::parse(&resolve_spec(spec, platform)).ok()
}

/// Which family of default bindings to build. A parameter rather than a
/// `cfg!` read inside [`Keymap::defaults`], so a test on one OS can check the
/// other's bindings too — a chord collision on Windows is otherwise invisible
/// from a Mac.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// ⌘ is the primary modifier.
    MacOs,
    /// Ctrl is the primary modifier.
    Windows,
    /// Ctrl is the primary modifier.
    Linux,
}

impl Platform {
    /// Every family, for a check that must hold wherever termherd runs.
    pub const ALL: [Platform; 3] = [Platform::MacOs, Platform::Windows, Platform::Linux];

    /// The family this build runs on. Any other Unix counts as Linux.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    const fn primary_mod(self) -> u8 {
        match self {
            Self::MacOs => MOD_CMD,
            Self::Windows | Self::Linux => MOD_CTRL,
        }
    }

    /// The primary modifier as chord syntax spells it: `cmd` or `ctrl`.
    const fn primary_name(self) -> &'static str {
        match self {
            Self::MacOs => "cmd",
            Self::Windows | Self::Linux => "ctrl",
        }
    }
}

/// Every built-in binding on `platform`, one pair per chord. Regular actions
/// come straight from the [`ACTIONS`] table's default chords; the
/// parameterized number row is listed here.
///
/// A list rather than a map, so a chord claimed twice stays visible to a test
/// instead of being settled silently by whichever pair came last.
#[must_use]
pub fn default_bindings(platform: Platform) -> Vec<(KeyChord, Action)> {
    let mut pairs: Vec<(KeyChord, Action)> = ACTIONS
        .iter()
        .flat_map(|def| {
            def.chords_for(platform)
                .iter()
                .filter_map(move |spec| default_chord(spec, platform))
                .map(move |chord| (chord, def.action))
        })
        .collect();
    // Jump straight to the Nth tab: ⌘1…⌘9 / Ctrl+1…Ctrl+9. The digit is
    // 1-based for the user; the action carries the 0-based index.
    pairs.extend((1..=NUMBER_ROW_TABS).map(|n| {
        (
            KeyChord::new(n.to_string(), platform.primary_mod()),
            Action::ActivateTab(n - 1),
        )
    }));
    pairs
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
    /// The built-in bindings of the platform this build runs on — see
    /// [`default_bindings`].
    pub fn defaults() -> Self {
        Self::defaults_for(Platform::current())
    }

    fn defaults_for(platform: Platform) -> Self {
        Keymap {
            bindings: default_bindings(platform).into_iter().collect(),
        }
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
    fn parse_spells_the_literal_plus_key_as_plus() {
        // "+" the key collides with "+" the separator, so specs write `plus`;
        // the parsed chord carries the literal character.
        assert_eq!(
            KeyChord::parse("ctrl+plus"),
            Ok(KeyChord::new("+", MOD_CTRL))
        );
        assert_eq!(
            KeyChord::parse("cmd+shift+plus"),
            Ok(KeyChord::new("+", MOD_CMD | MOD_SHIFT))
        );
    }

    #[test]
    fn parse_rejects_missing_or_doubled_keys() {
        assert_eq!(KeyChord::parse("ctrl+"), Err(ChordError::NoKey));
        assert_eq!(KeyChord::parse("ctrl+shift"), Err(ChordError::NoKey));
        assert_eq!(KeyChord::parse("a+b"), Err(ChordError::MultipleKeys));
    }

    #[test]
    fn defaults_bind_the_splits_per_platform() {
        let mac = Keymap::defaults_for(Platform::MacOs);
        assert_eq!(
            mac.lookup(&KeyChord::new("d", MOD_CMD)),
            Some(Action::SplitVertical)
        );
        assert_eq!(
            mac.lookup(&KeyChord::new("d", MOD_CMD | MOD_SHIFT)),
            Some(Action::SplitHorizontal)
        );
        for platform in [Platform::Windows, Platform::Linux] {
            let map = Keymap::defaults_for(platform);
            for key in ["=", "+"] {
                assert_eq!(
                    map.lookup(&KeyChord::new(key, MOD_ALT | MOD_SHIFT)),
                    Some(Action::SplitVertical),
                    "alt+shift+{key} on {platform:?}"
                );
            }
            for key in ["-", "d"] {
                assert_eq!(
                    map.lookup(&KeyChord::new(key, MOD_ALT | MOD_SHIFT)),
                    Some(Action::SplitHorizontal),
                    "alt+shift+{key} on {platform:?}"
                );
            }
            assert_eq!(map.lookup(&KeyChord::new("d", MOD_CTRL)), None);
        }
    }

    #[test]
    fn defaults_resolve_copy_and_paste_for_the_platform() {
        let mac = Keymap::defaults_for(Platform::MacOs);
        assert_eq!(mac.lookup(&KeyChord::new("c", MOD_CMD)), Some(Action::Copy));
        assert_eq!(
            mac.lookup(&KeyChord::new("v", MOD_CMD)),
            Some(Action::Paste)
        );
        for (platform, bare_ctrl_v) in [
            (Platform::Windows, Some(Action::Paste)),
            (Platform::Linux, None),
        ] {
            let map = Keymap::defaults_for(platform);
            let ctrl_shift = |key| map.lookup(&KeyChord::new(key, MOD_CTRL | MOD_SHIFT));
            assert_eq!(ctrl_shift("c"), Some(Action::Copy));
            assert_eq!(ctrl_shift("v"), Some(Action::Paste));
            assert_eq!(map.lookup(&KeyChord::new("v", MOD_CTRL)), bare_ctrl_v);
        }
        for platform in Platform::ALL {
            let map = Keymap::defaults_for(platform);
            // Tab cycling is bound on every platform.
            assert_eq!(
                map.lookup(&KeyChord::new("tab", MOD_CTRL)),
                Some(Action::NextTab)
            );
            // An unbound chord resolves to nothing.
            assert_eq!(map.lookup(&KeyChord::new("q", MOD_CTRL)), None);
        }
    }

    #[test]
    fn toggle_sidebar_binds_to_the_platform_b_chord() {
        assert_eq!(
            Keymap::defaults_for(Platform::MacOs).lookup(&KeyChord::new("b", MOD_CMD)),
            Some(Action::ToggleSidebar)
        );
        assert_eq!(
            Keymap::defaults_for(Platform::Linux).lookup(&KeyChord::new("b", MOD_CTRL | MOD_SHIFT)),
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
        // the same name, so name↔action can never drift.
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
    fn action_catalog_mirrors_the_source_table() {
        // The public catalogue is the read-only face of ACTIONS: every entry,
        // with each platform's defaults as chords `KeyChord::parse` accepts and
        // the keymap binds.
        for platform in Platform::ALL {
            let catalog = action_catalog(platform);
            assert_eq!(catalog.len(), ACTIONS.len());
            for (entry, def) in catalog.iter().zip(ACTIONS.iter()) {
                assert_eq!(entry.name, def.name);
                let published: Vec<KeyChord> = entry
                    .default_chords
                    .iter()
                    .map(|spec| KeyChord::parse(spec).expect("a published default parses"))
                    .collect();
                let bound: Vec<KeyChord> = default_bindings(platform)
                    .into_iter()
                    .filter(|(_, action)| *action == def.action)
                    .map(|(chord, _)| chord)
                    .collect();
                assert_eq!(published, bound, "{} on {platform:?}", def.name);
            }
        }
    }

    #[test]
    fn every_default_chord_spec_in_the_table_parses() {
        // The specs are authored in-tree; a typo would silently drop a default
        // binding (defaults() skips unparsable specs to keep core panic-free).
        // Fail here instead so it never ships.
        for platform in Platform::ALL {
            for def in ACTIONS {
                for spec in def.chords_for(platform) {
                    assert!(
                        default_chord(spec, platform).is_some(),
                        "default chord spec `{spec}` for {:?} must parse on {platform:?}",
                        def.action
                    );
                }
            }
        }
    }

    #[test]
    fn every_action_round_trips_through_its_name() {
        // `name()` is what a surface reporting *which* action ran spells, so it
        // must be total and it must round-trip — otherwise a reported name is
        // not a name a caller can send back. This is the property `config_name`
        // cannot have: it returns None for the parameterized family.
        for def in ACTIONS {
            let name = def.action.name();
            assert_eq!(name, def.name, "a table action names itself from the table");
            assert_eq!(
                Action::from_config_name(&name),
                Some(def.action),
                "{name} must round-trip"
            );
        }
        for index in 0..NUMBER_ROW_TABS {
            let action = Action::ActivateTab(index);
            let name = action.name();
            assert_eq!(
                Action::from_config_name(&name),
                Some(action),
                "{name} must round-trip"
            );
        }
    }

    #[test]
    fn activate_tab_names_carry_the_one_based_digit_the_user_sees() {
        // The action holds a 0-based index; the name holds the 1-based digit, so
        // `activate-tab-1` is the first tab. Off by one here would rename every
        // tab jump on the wire.
        assert_eq!(Action::ActivateTab(0).name(), "activate-tab-1");
        assert_eq!(Action::ActivateTab(8).name(), "activate-tab-9");
    }

    #[test]
    fn every_default_bound_action_is_nameable() {
        // The stronger form of `every_default_bound_action_is_reconfigurable`:
        // not just "has some name" but "names itself back to itself". This is
        // what keeps `name()`'s fallback arm out of reach.
        let map = Keymap::defaults();
        for action in map.bindings.values() {
            assert_eq!(
                Action::from_config_name(&action.name()),
                Some(*action),
                "default-bound {action:?} must round-trip through its name"
            );
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
            assert!(named, "default-bound {action:?} has no config name");
        }
    }

    #[test]
    fn defaults_bind_scroll_top_and_bottom_per_platform() {
        let mac = Keymap::defaults_for(Platform::MacOs);
        assert_eq!(
            mac.lookup(&KeyChord::new("up", MOD_CMD)),
            Some(Action::ScrollTop)
        );
        assert_eq!(
            mac.lookup(&KeyChord::new("down", MOD_CMD)),
            Some(Action::ScrollBottom)
        );
        // Elsewhere Ctrl+Up / Ctrl+Down stay the program's cursor keys.
        let other = Keymap::defaults_for(Platform::Linux);
        assert_eq!(
            other.lookup(&KeyChord::new("home", MOD_CTRL | MOD_SHIFT)),
            Some(Action::ScrollTop)
        );
        assert_eq!(
            other.lookup(&KeyChord::new("end", MOD_CTRL | MOD_SHIFT)),
            Some(Action::ScrollBottom)
        );
        assert_eq!(other.lookup(&KeyChord::new("up", MOD_CTRL)), None);
        assert_eq!(other.lookup(&KeyChord::new("down", MOD_CTRL)), None);
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
    fn defaults_bind_the_context_tab_chords_to_t() {
        // All three hang off T; off macOS bare Ctrl+T stays the terminal's.
        let mac = Keymap::defaults_for(Platform::MacOs);
        let other = Keymap::defaults_for(Platform::Linux);
        for (map, mods, action) in [
            (&mac, MOD_CMD, Action::NewShellHere),
            (&mac, MOD_CMD | MOD_ALT, Action::NewClaudeSessionHere),
            (&mac, MOD_CMD | MOD_SHIFT, Action::ReopenClosedTab),
            (&other, MOD_CTRL | MOD_SHIFT, Action::NewShellHere),
            (&other, MOD_CTRL | MOD_ALT, Action::NewClaudeSessionHere),
            (
                &other,
                MOD_CTRL | MOD_ALT | MOD_SHIFT,
                Action::ReopenClosedTab,
            ),
        ] {
            assert_eq!(map.lookup(&KeyChord::new("t", mods)), Some(action));
        }
        assert_eq!(other.lookup(&KeyChord::new("t", MOD_CTRL)), None);
        // The three names answer in the config vocabulary too.
        assert_eq!(
            Action::from_config_name("new-shell-here"),
            Some(Action::NewShellHere)
        );
        assert_eq!(
            Action::from_config_name("new-claude-session-here"),
            Some(Action::NewClaudeSessionHere)
        );
        assert_eq!(
            Action::from_config_name("reopen-closed-tab"),
            Some(Action::ReopenClosedTab)
        );
    }

    #[test]
    fn defaults_bind_capture_to_the_primary_modifier_shift_s() {
        // ⌘⇧S (macOS) / Ctrl+Shift+S triggers the state+PNG capture, and
        // the action answers to `capture` in the config vocabulary.
        let map = Keymap::defaults();
        assert_eq!(
            map.lookup(&KeyChord::new("s", primary_mod() | MOD_SHIFT)),
            Some(Action::Capture)
        );
        assert_eq!(Action::from_config_name("capture"), Some(Action::Capture));
    }

    #[test]
    fn defaults_bind_toggle_record_to_the_primary_modifier_shift_r() {
        // ⌘⇧R (macOS) / Ctrl+Shift+R toggles the GIF screencast, and the
        // action answers to `toggle-record` in the config vocabulary.
        let map = Keymap::defaults();
        assert_eq!(
            map.lookup(&KeyChord::new("r", primary_mod() | MOD_SHIFT)),
            Some(Action::ToggleRecord)
        );
        assert_eq!(
            Action::from_config_name("toggle-record"),
            Some(Action::ToggleRecord)
        );
    }

    #[test]
    fn defaults_bind_rename_tab_to_the_primary_modifier_shift_i_on_every_platform() {
        assert_eq!(
            Keymap::defaults_for(Platform::MacOs).lookup(&KeyChord::new("i", MOD_CMD | MOD_SHIFT)),
            Some(Action::RenameTab)
        );
        assert_eq!(
            Keymap::defaults_for(Platform::Linux).lookup(&KeyChord::new("i", MOD_CTRL | MOD_SHIFT)),
            Some(Action::RenameTab)
        );
    }

    #[test]
    fn defaults_bind_open_tab_menu_to_the_primary_modifier_shift_m_on_every_platform() {
        assert_eq!(
            Keymap::defaults_for(Platform::MacOs).lookup(&KeyChord::new("m", MOD_CMD | MOD_SHIFT)),
            Some(Action::OpenTabMenu)
        );
        assert_eq!(
            Keymap::defaults_for(Platform::Linux).lookup(&KeyChord::new("m", MOD_CTRL | MOD_SHIFT)),
            Some(Action::OpenTabMenu)
        );
        assert_eq!(
            Action::from_config_name("open-tab-menu"),
            Some(Action::OpenTabMenu)
        );
    }

    #[test]
    fn no_default_chord_is_claimed_by_two_actions_on_either_platform() {
        // The keymap is a map, so a collision would be settled silently by
        // whichever binding came last. Both platforms, because a Ctrl-only
        // collision is invisible from a Mac.
        for platform in Platform::ALL {
            let mut seen: HashMap<KeyChord, Action> = HashMap::new();
            for (chord, action) in default_bindings(platform) {
                let earlier = seen.insert(chord.clone(), action);
                assert_eq!(
                    earlier, None,
                    "{chord:?} is also bound to {action:?} on {platform:?}"
                );
            }
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
        // The row stops at nine — zero belongs to zoom-reset, not a
        // tenth tab jump.
        assert_eq!(
            map.lookup(&KeyChord::new("0", primary_mod())),
            Some(Action::ZoomReset)
        );
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
