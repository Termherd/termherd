//! Thin user settings (FR10) — shell profile and GUI theme, persisted to
//! `~/.termherd/settings.json`. A file adapter owned by the shell, like
//! [`crate::window_config`]; `core` never sees it. Window bounds keep their
//! own `window.json` (FR12).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
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
