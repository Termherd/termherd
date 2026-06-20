//! Thin user settings (FR10) — shell profile and GUI theme, persisted to
//! `~/.termherd/settings.json`. A file adapter owned by the shell, like
//! [`crate::window_config`]; `core` never sees it. Window bounds keep their
//! own `window.json` (FR12).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use termherd_core::{Action, KeyChord, Keymap};
use tracing::warn;

/// The persisted user settings. Every field defaults, so a missing or partial
/// file still yields a usable config — settings must never block startup.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Shell to launch for each session; absent → the platform default login
    /// shell.
    pub shell: Option<ShellProfile>,
    /// GUI chrome theme (the terminal grid keeps its own colours).
    pub theme: ThemeChoice,
    /// Keyboard overrides: action name (kebab-case) → one chord or a list of
    /// chords. Each entry replaces that action's platform default (FR9). Same
    /// table on every OS; unspecified actions keep their per-platform default.
    pub keys: HashMap<String, ChordList>,
}

/// One or several chords bound to an action — a bare string for the common
/// single-binding case, or an array for several.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChordList {
    One(String),
    Many(Vec<String>),
}

impl ChordList {
    fn iter(&self) -> impl Iterator<Item = &str> {
        match self {
            ChordList::One(s) => std::slice::from_ref(s).iter(),
            ChordList::Many(v) => v.iter(),
        }
        .map(String::as_str)
    }
}

/// A shell to spawn instead of the platform default (e.g. `pwsh`, `bash`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellProfile {
    /// The program to run.
    pub program: String,
    /// Arguments passed to the program.
    #[serde(default)]
    pub args: Vec<String>,
}

/// Which iced theme dresses the GUI chrome (sidebar, tab strip, buttons).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    /// A dark chrome, matching the terminal's dark background.
    #[default]
    Dark,
    /// A light chrome.
    Light,
}

impl ThemeChoice {
    /// The iced theme this choice maps to.
    #[must_use]
    pub fn to_iced(self) -> iced::Theme {
        match self {
            ThemeChoice::Dark => iced::Theme::Dark,
            ThemeChoice::Light => iced::Theme::Light,
        }
    }
}

impl Settings {
    /// The active keymap: platform defaults with the user's `keys` overrides
    /// applied (FR9). Unknown action names and unparsable chords are logged and
    /// skipped, so a typo never breaks the rest of the bindings.
    #[must_use]
    pub fn keymap(&self) -> Keymap {
        let mut keymap = Keymap::defaults();
        for (name, list) in &self.keys {
            let Some(action) = Action::from_config_name(name) else {
                warn!(action = name, "unknown key action in settings; ignoring");
                continue;
            };
            let mut chords = Vec::new();
            for raw in list.iter() {
                match KeyChord::parse(raw) {
                    Ok(chord) => chords.push(chord),
                    Err(e) => warn!(chord = raw, error = %e, "invalid chord; ignoring"),
                }
            }
            // All chords invalid → leave the default binding untouched.
            if !chords.is_empty() {
                keymap.set(action, chords);
            }
        }
        keymap
    }

    /// Load persisted settings; any problem (no file, bad JSON) falls back to
    /// defaults — a corrupt config must never prevent startup.
    #[must_use]
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
                warn!(error = %e, path = %path.display(), "invalid settings; using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }
}

/// `~/.termherd/settings.json` — the app data dir from the PRD (§7).
fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".termherd").join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_a_default_shell_and_dark_theme() {
        let s = Settings::default();
        assert!(s.shell.is_none());
        assert_eq!(s.theme, ThemeChoice::Dark);
    }

    #[test]
    fn deserialises_a_partial_file_filling_in_defaults() {
        // Only the shell is given; the theme falls back to its default.
        let s: Settings =
            serde_json::from_str(r#"{ "shell": { "program": "pwsh" } }"#).expect("valid json");
        let shell = s.shell.expect("a shell");
        assert_eq!(shell.program, "pwsh");
        assert!(shell.args.is_empty());
        assert_eq!(s.theme, ThemeChoice::Dark);
    }

    #[test]
    fn theme_round_trips_through_json_lowercased() {
        let json = serde_json::to_string(&ThemeChoice::Light).expect("serialise");
        assert_eq!(json, "\"light\"");
        let back: ThemeChoice = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back, ThemeChoice::Light);
    }

    #[test]
    fn keys_override_a_default_binding() {
        use termherd_core::{Action, KeyChord, keymap::MOD_CTRL};
        let s: Settings =
            serde_json::from_str(r#"{ "keys": { "copy": "ctrl+y" } }"#).expect("valid json");
        let map = s.keymap();
        // The override takes effect…
        assert_eq!(
            map.lookup(&KeyChord::new("y", MOD_CTRL)),
            Some(Action::Copy)
        );
        // …and a binding we did not touch keeps its default.
        assert_eq!(
            map.lookup(&KeyChord::new("tab", MOD_CTRL)),
            Some(Action::NextTab)
        );
    }

    #[test]
    fn keys_can_bind_a_number_row_tab_jump() {
        // A user remaps the third-tab jump to a non-default chord (issue #26).
        use termherd_core::{Action, KeyChord};
        let s: Settings = serde_json::from_str(r#"{ "keys": { "activate-tab-3": "alt+3" } }"#)
            .expect("valid json");
        let map = s.keymap();
        assert_eq!(
            map.lookup(&KeyChord::parse("alt+3").expect("valid chord")),
            Some(Action::ActivateTab(2))
        );
    }

    #[test]
    fn a_list_binds_several_chords_and_bad_entries_are_skipped() {
        use termherd_core::{Action, KeyChord, keymap::MOD_CTRL};
        let s: Settings =
            serde_json::from_str(r#"{ "keys": { "paste": ["ctrl+y", "not a chord"] } }"#)
                .expect("valid json");
        let map = s.keymap();
        assert_eq!(
            map.lookup(&KeyChord::new("y", MOD_CTRL)),
            Some(Action::Paste)
        );
    }

    #[test]
    fn shell_args_deserialise() {
        let s: Settings = serde_json::from_str(
            r#"{ "shell": { "program": "bash", "args": ["-l"] }, "theme": "light" }"#,
        )
        .expect("valid json");
        let shell = s.shell.expect("a shell");
        assert_eq!(shell.program, "bash");
        assert_eq!(shell.args, vec!["-l".to_string()]);
        assert_eq!(s.theme, ThemeChoice::Light);
    }
}
