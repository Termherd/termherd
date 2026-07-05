//! Thin user settings (FR10) — shell profile and GUI theme, persisted to
//! `~/.termherd/settings.json`. A file adapter owned by the shell, like
//! [`crate::window_config`]; `core` never sees it. Window bounds keep their
//! own `window.json` (FR12).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use termherd_core::{Action, KeyChord, Keymap};
use tracing::warn;

use crate::record::RecordConfig;

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
    /// GIF screencast budget: frames per second, the duration cap,
    /// and the frame scale. Absent → the built-in default.
    pub record: RecordSettings,
    /// Sidebar behaviour: how many sessions each project lists before
    /// folding the tail behind an expander. Absent → the built-in default.
    pub sidebar: SidebarSettings,
    /// Terminal appearance: the base font size the zoom steps from.
    /// Absent → the built-in default.
    pub terminal: TerminalSettings,
    /// Close-confirmation policy: whether closing a tab or quitting the app
    /// prompts first. Absent → both confirm only while a session runs a
    /// foreground process, and close/quit an all-idle target silently.
    pub close: CloseSettings,
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

/// The on-disk GIF screencast budget. Each field defaults to the
/// built-in [`RecordConfig::default`], so a missing or partial `record` block
/// keeps current behaviour. Raw values are sanitised into a [`RecordConfig`] by
/// [`RecordSettings::into_config`].
///
/// The fields are deliberately **wide** (`i64`/`f64`): the runtime budget is
/// `u32`/`f32`, but parsing into those would make an out-of-range typo (an extra
/// digit, a negative) fail serde for the *whole* `settings.json` — silently
/// resetting the user's keymap, theme and shell too. Parsing wide then clamping
/// keeps a bad `record` value contained to the record budget.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordSettings {
    /// Frames captured per second.
    pub fps: i64,
    /// Hard cap on recording length, in seconds.
    pub max_seconds: i64,
    /// Frame downscale factor (1.0 = full window, 0.5 = half).
    pub scale: f64,
}

impl Default for RecordSettings {
    /// Mirrors [`RecordConfig::default`], so the two never drift (asserted in a
    /// test).
    fn default() -> Self {
        let d = RecordConfig::default();
        Self {
            fps: i64::from(d.fps),
            max_seconds: i64::from(d.max_seconds),
            scale: f64::from(d.scale),
        }
    }
}

/// Sane bounds for the record budget — wide enough to be useful, tight enough
/// that a typo (`scale: 50`, `max_seconds: 0`) can't wedge the encoder.
const FPS_RANGE: (u32, u32) = (1, 60);
const SECONDS_RANGE: (u32, u32) = (1, 600);
const SCALE_RANGE: (f32, f32) = (0.1, 1.0);

impl RecordSettings {
    /// Sanitise the raw values into a [`RecordConfig`]: clamp each into its
    /// range, and fall back to the default scale if it is not finite (a NaN
    /// can't arrive from JSON, but the runtime type must never carry one). The
    /// wide→narrow clamp also absorbs out-of-`u32`-range typos that would
    /// otherwise have failed the whole-file parse.
    #[must_use]
    pub fn into_config(self) -> RecordConfig {
        let clamp_to_u32 =
            |v: i64, range: (u32, u32)| v.clamp(i64::from(range.0), i64::from(range.1)) as u32;
        let scale = if self.scale.is_finite() {
            self.scale
                .clamp(f64::from(SCALE_RANGE.0), f64::from(SCALE_RANGE.1)) as f32
        } else {
            RecordConfig::default().scale
        };
        RecordConfig {
            fps: clamp_to_u32(self.fps, FPS_RANGE),
            max_seconds: clamp_to_u32(self.max_seconds, SECONDS_RANGE),
            scale,
        }
    }
}

/// The on-disk sidebar settings. Wide (`i64`) for the same reason as
/// [`RecordSettings`]: an out-of-range typo must not fail serde for the whole
/// file and silently reset the user's other settings.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct SidebarSettings {
    /// Sessions shown per project before the tail folds behind an expander;
    /// `0` shows every session (the earlier behaviour).
    pub session_limit: i64,
}

/// Sessions shown per project by default.
const DEFAULT_SESSION_LIMIT: i64 = 5;
/// Bound for the limit — anything above is effectively "show all", and the
/// clamp absorbs out-of-range typos (a negative folds to 0 = show all).
const SESSION_LIMIT_MAX: i64 = 10_000;

impl Default for SidebarSettings {
    fn default() -> Self {
        Self {
            session_limit: DEFAULT_SESSION_LIMIT,
        }
    }
}

impl SidebarSettings {
    /// Sanitise the raw value into the runtime limit: clamp into range, so a
    /// negative typo means "show all" rather than failing the file.
    #[must_use]
    pub fn limit(self) -> usize {
        self.session_limit.clamp(0, SESSION_LIMIT_MAX) as usize
    }
}

/// The on-disk terminal settings. Wide (`f64`) for the same reason as
/// [`RecordSettings`]: an out-of-range typo must not fail serde for the whole
/// file and silently reset the user's other settings.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalSettings {
    /// Base terminal font size in pixels; the zoom shortcuts step from
    /// here at runtime without touching this setting.
    pub font_size: f64,
}

/// Mirrors `core`'s built-in default so an absent block changes nothing
/// (asserted in a test) and the effective range core clamps into.
const FONT_SIZE_RANGE: (f32, f32) = (6.0, 40.0);

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font_size: f64::from(termherd_core::DEFAULT_FONT_SIZE),
        }
    }
}

impl TerminalSettings {
    /// Sanitise the raw value into the runtime base size: clamp into range,
    /// and fall back to the default if it is not finite.
    #[must_use]
    pub fn font_size(self) -> f32 {
        if self.font_size.is_finite() {
            self.font_size
                .clamp(f64::from(FONT_SIZE_RANGE.0), f64::from(FONT_SIZE_RANGE.1))
                as f32
        } else {
            termherd_core::DEFAULT_FONT_SIZE
        }
    }
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

/// When a close that would terminate running session(s) asks for confirmation
/// first. Shared vocabulary for both the tab close and the app quit; each gets
/// its own policy via [`CloseSettings`].
///
/// The variants serialise camel-cased — `"alwaysConfirm"`, `"confirmWhenActive"`,
/// `"noConfirmation"` — so the value reads as its own sentence in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfirmClose {
    /// Always prompt before closing.
    AlwaysConfirm,
    /// Prompt only while the target is still *active* — running a foreground
    /// process worth confirming (a working shell or any live Claude). An idle
    /// target closes silently. The caller supplies what "active" means: a tab
    /// close keys off its own sessions, a quit off every session.
    ConfirmWhenActive,
    /// Never prompt — close immediately.
    NoConfirmation,
}

impl ConfirmClose {
    /// Whether a close must be confirmed first, given whether the target is
    /// still `active` (has work worth confirming — the caller decides what
    /// counts). The single decision seam, kept pure so the tab and quit paths —
    /// and their tests — share one truth table.
    #[must_use]
    pub fn confirms(self, active: bool) -> bool {
        match self {
            ConfirmClose::AlwaysConfirm => true,
            ConfirmClose::ConfirmWhenActive => active,
            ConfirmClose::NoConfirmation => false,
        }
    }
}

/// Per-action close-confirmation policy. Each field defaults to the behaviour
/// already shipped, so an absent (or partial) `close` block changes nothing:
/// both a tab close and a quit confirm only while a session runs a foreground
/// process — an all-idle target closes / quits silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CloseSettings {
    /// Closing a tab (and hard-killing its session(s)).
    pub tab: ConfirmClose,
    /// Quitting the app (hard-killing every live session).
    pub app: ConfirmClose,
}

impl Default for CloseSettings {
    fn default() -> Self {
        Self {
            tab: ConfirmClose::ConfirmWhenActive,
            app: ConfirmClose::ConfirmWhenActive,
        }
    }
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
    /// The sanitised GIF screencast budget, clamped into its ranges.
    #[must_use]
    pub fn record_config(&self) -> RecordConfig {
        self.record.into_config()
    }

    /// The sanitised sidebar session limit; `0` shows every session.
    #[must_use]
    pub fn session_limit(&self) -> usize {
        self.sidebar.limit()
    }

    /// The sanitised terminal base font size, clamped into its range.
    #[must_use]
    pub fn font_size(&self) -> f32 {
        self.terminal.font_size()
    }

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
    Some(crate::paths::termherd_dir()?.join("settings.json"))
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
    fn record_defaults_match_the_built_in_record_config() {
        // The settings default must mirror RecordConfig::default so an absent
        // `record` block changes nothing.
        let from_settings = Settings::default().record_config();
        let built_in = RecordConfig::default();
        assert_eq!(from_settings.fps, built_in.fps);
        assert_eq!(from_settings.max_seconds, built_in.max_seconds);
        assert!((from_settings.scale - built_in.scale).abs() < f32::EPSILON);
    }

    #[test]
    fn record_block_overrides_and_a_partial_block_keeps_defaults() {
        // A full block is taken verbatim (within range)…
        let full: Settings =
            serde_json::from_str(r#"{ "record": { "fps": 15, "max_seconds": 10, "scale": 1.0 } }"#)
                .expect("valid json");
        let c = full.record_config();
        assert_eq!((c.fps, c.max_seconds), (15, 10));
        assert!((c.scale - 1.0).abs() < f32::EPSILON);

        // …and a partial block fills the rest from the default.
        let partial: Settings =
            serde_json::from_str(r#"{ "record": { "fps": 5 } }"#).expect("valid json");
        let c = partial.record_config();
        assert_eq!((c.fps, c.max_seconds), (5, 30));
        assert!((c.scale - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn record_values_are_clamped_into_range() {
        // A typo can't wedge the encoder: 0 fps, an absurd cap, and an
        // out-of-range scale all clamp instead of taking effect.
        let s: Settings = serde_json::from_str(
            r#"{ "record": { "fps": 0, "max_seconds": 99999, "scale": 5.0 } }"#,
        )
        .expect("valid json");
        let c = s.record_config();
        assert_eq!(c.fps, 1, "fps floors at 1");
        assert_eq!(c.max_seconds, 600, "the cap is bounded");
        assert!((c.scale - 1.0).abs() < f32::EPSILON, "scale ceils at 1.0");

        // A negative scale clamps up to the floor.
        let s: Settings =
            serde_json::from_str(r#"{ "record": { "scale": -1.0 } }"#).expect("valid json");
        assert!((s.record_config().scale - 0.1).abs() < f32::EPSILON);
    }

    #[test]
    fn an_out_of_u32_range_record_typo_does_not_reset_the_whole_file() {
        // Regression for the code-review finding: an extra-digit `max_seconds`
        // (> u32::MAX) or a negative `fps` used to fail serde for the ENTIRE
        // file, silently discarding keymap/theme/shell. Wide parse + clamp keeps
        // it contained — the file still loads, the rest of the settings survive.
        let s: Settings = serde_json::from_str(
            r#"{ "theme": "light", "record": { "max_seconds": 9999999999, "fps": -1 } }"#,
        )
        .expect("an out-of-u32 record value must not fail the whole parse");
        assert_eq!(s.theme, ThemeChoice::Light, "the rest of the file survives");
        let c = s.record_config();
        assert_eq!(c.max_seconds, 600, "an absurd cap clamps, not resets");
        assert_eq!(c.fps, 1, "a negative fps clamps to the floor");
    }

    #[test]
    fn sidebar_limit_defaults_overrides_and_clamps() {
        // Absent block → the built-in default of 5.
        assert_eq!(Settings::default().session_limit(), 5);

        // An explicit value is taken…
        let s: Settings =
            serde_json::from_str(r#"{ "sidebar": { "session_limit": 12 } }"#).expect("valid json");
        assert_eq!(s.session_limit(), 12);

        // …0 disables truncation…
        let s: Settings =
            serde_json::from_str(r#"{ "sidebar": { "session_limit": 0 } }"#).expect("valid json");
        assert_eq!(s.session_limit(), 0);

        // …and a negative typo folds to "show all" without failing the file.
        let s: Settings =
            serde_json::from_str(r#"{ "theme": "light", "sidebar": { "session_limit": -3 } }"#)
                .expect("a bad sidebar value must not fail the whole parse");
        assert_eq!(s.session_limit(), 0);
        assert_eq!(s.theme, ThemeChoice::Light, "the rest of the file survives");
    }

    #[test]
    fn terminal_font_size_defaults_overrides_and_clamps() {
        // Absent block → core's built-in default, so nothing changes.
        let d = Settings::default().font_size();
        assert!((d - termherd_core::DEFAULT_FONT_SIZE).abs() < f32::EPSILON);

        // An explicit value is taken…
        let s: Settings =
            serde_json::from_str(r#"{ "terminal": { "font_size": 18 } }"#).expect("valid json");
        assert!((s.font_size() - 18.0).abs() < f32::EPSILON);

        // …and typos clamp instead of failing the whole file.
        let s: Settings =
            serde_json::from_str(r#"{ "theme": "light", "terminal": { "font_size": 400 } }"#)
                .expect("a bad font size must not fail the whole parse");
        assert!((s.font_size() - 40.0).abs() < f32::EPSILON);
        assert_eq!(s.theme, ThemeChoice::Light, "the rest of the file survives");
        let s: Settings =
            serde_json::from_str(r#"{ "terminal": { "font_size": -3 } }"#).expect("valid json");
        assert!((s.font_size() - 6.0).abs() < f32::EPSILON);
    }

    #[test]
    fn close_defaults_preserve_the_shipped_behaviour() {
        // Absent `close` block: both a tab close and a quit confirm only while
        // active — exactly what was hard-coded before the setting.
        let c = Settings::default().close;
        assert_eq!(c.tab, ConfirmClose::ConfirmWhenActive);
        assert_eq!(c.app, ConfirmClose::ConfirmWhenActive);
    }

    #[test]
    fn confirm_close_truth_table() {
        use ConfirmClose::{AlwaysConfirm, ConfirmWhenActive, NoConfirmation};
        // Always prompts, active or not.
        assert!(AlwaysConfirm.confirms(true) && AlwaysConfirm.confirms(false));
        // Prompts only while a session is live.
        assert!(ConfirmWhenActive.confirms(true) && !ConfirmWhenActive.confirms(false));
        // Never prompts.
        assert!(!NoConfirmation.confirms(true) && !NoConfirmation.confirms(false));
    }

    #[test]
    fn close_block_deserialises_camel_cased_values() {
        let s: Settings = serde_json::from_str(
            r#"{ "close": { "tab": "confirmWhenActive", "app": "noConfirmation" } }"#,
        )
        .expect("valid json");
        assert_eq!(s.close.tab, ConfirmClose::ConfirmWhenActive);
        assert_eq!(s.close.app, ConfirmClose::NoConfirmation);
    }

    #[test]
    fn a_partial_close_block_keeps_the_other_field_at_its_default() {
        // Only `app` given → `tab` keeps its confirm-when-active default.
        let s: Settings =
            serde_json::from_str(r#"{ "close": { "app": "alwaysConfirm" } }"#).expect("valid json");
        assert_eq!(s.close.tab, ConfirmClose::ConfirmWhenActive);
        assert_eq!(s.close.app, ConfirmClose::AlwaysConfirm);
    }

    #[test]
    fn confirm_close_round_trips_camel_cased() {
        let json = serde_json::to_string(&ConfirmClose::ConfirmWhenActive).expect("serialise");
        assert_eq!(json, "\"confirmWhenActive\"");
        let back: ConfirmClose = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back, ConfirmClose::ConfirmWhenActive);
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
        // A user remaps the third-tab jump to a non-default chord.
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
