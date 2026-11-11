//! Where termherd and Claude Code keep their files, resolved from an injected
//! environment lookup.
//!
//! Every binary reads these locations — the GUI, the scanner it builds, the
//! stdio MCP server writing the same `settings.json` — so the precedence is
//! one pure function rather than a copy per crate. The caller passes
//! `|name| std::env::var_os(name)` from its `main()`; tests pass a table.
//!
//! An override set to the empty string counts as unset, the usual way to
//! clear a variable for one command line.

use std::ffi::OsString;
use std::path::PathBuf;

/// Relocates termherd's own data dir (settings, layout, bridge file), and
/// nothing else: a dev build isolated with it keeps the user's shell home.
pub const DATA_DIR_VAR: &str = "TERMHERD_DATA_DIR";

/// Claude Code's own relocation of `~/.claude`, honoured with the same
/// precedence so termherd reads the directory the sessions it launches write.
pub const CLAUDE_CONFIG_DIR_VAR: &str = "CLAUDE_CONFIG_DIR";

/// The user's home directory: `%USERPROFILE%` (Windows) then `$HOME` (Unix).
/// `None` when neither is set.
#[must_use]
pub fn home(env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    env("USERPROFILE")
        .or_else(|| env("HOME"))
        .map(PathBuf::from)
}

/// termherd's data dir: [`DATA_DIR_VAR`], else `<home>/.termherd`.
#[must_use]
pub fn data_dir(env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    overridden(env, DATA_DIR_VAR).or_else(|| home(env).map(|home| home.join(".termherd")))
}

/// Claude Code's data dir: [`CLAUDE_CONFIG_DIR_VAR`], else `<home>/.claude`.
#[must_use]
pub fn claude_dir(env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    overridden(env, CLAUDE_CONFIG_DIR_VAR).or_else(|| home(env).map(|home| home.join(".claude")))
}

fn overridden(env: &impl Fn(&str) -> Option<OsString>, name: &str) -> Option<PathBuf> {
    env(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let pairs: Vec<(String, OsString)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn userprofile_wins_over_home() {
        let env = env_of(&[("USERPROFILE", "/win-home"), ("HOME", "/unix-home")]);
        assert_eq!(home(&env), Some(PathBuf::from("/win-home")));
    }

    #[test]
    fn home_is_the_fallback_and_nothing_is_none() {
        assert_eq!(home(&env_of(&[("HOME", "/h")])), Some(PathBuf::from("/h")));
        assert_eq!(home(&env_of(&[])), None);
        assert_eq!(data_dir(&env_of(&[])), None);
        assert_eq!(claude_dir(&env_of(&[])), None);
    }

    #[test]
    fn without_overrides_both_dirs_sit_in_home() {
        let env = env_of(&[("HOME", "/h")]);
        assert_eq!(data_dir(&env), Some(PathBuf::from("/h/.termherd")));
        assert_eq!(claude_dir(&env), Some(PathBuf::from("/h/.claude")));
    }

    #[test]
    fn each_override_moves_its_own_dir_only() {
        let env = env_of(&[("HOME", "/h"), (DATA_DIR_VAR, "/dev/data")]);
        assert_eq!(data_dir(&env), Some(PathBuf::from("/dev/data")));
        assert_eq!(claude_dir(&env), Some(PathBuf::from("/h/.claude")));

        let env = env_of(&[("HOME", "/h"), (CLAUDE_CONFIG_DIR_VAR, "/fixture")]);
        assert_eq!(data_dir(&env), Some(PathBuf::from("/h/.termherd")));
        assert_eq!(claude_dir(&env), Some(PathBuf::from("/fixture")));
    }

    #[test]
    fn an_override_needs_no_home() {
        let env = env_of(&[(DATA_DIR_VAR, "/d"), (CLAUDE_CONFIG_DIR_VAR, "/c")]);
        assert_eq!(data_dir(&env), Some(PathBuf::from("/d")));
        assert_eq!(claude_dir(&env), Some(PathBuf::from("/c")));
    }

    #[test]
    fn an_empty_override_counts_as_unset() {
        let env = env_of(&[
            ("HOME", "/h"),
            (DATA_DIR_VAR, ""),
            (CLAUDE_CONFIG_DIR_VAR, ""),
        ]);
        assert_eq!(data_dir(&env), Some(PathBuf::from("/h/.termherd")));
        assert_eq!(claude_dir(&env), Some(PathBuf::from("/h/.claude")));
    }
}
