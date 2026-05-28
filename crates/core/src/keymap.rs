//! Keymap — `KeyChord -> Action`.
//!
//! Stub in M0. Real bindings (configurable, TOML-loaded; see OQ2 in the PRD)
//! land in M3 along with `F-keyboard-shortcuts`.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub key: String,
    /// Bitfield: `Ctrl = 1`, `Alt = 2`, `Shift = 4`, `Cmd/Super = 8`.
    pub mods: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
}

#[derive(Debug, Clone, Default)]
pub struct Keymap;
