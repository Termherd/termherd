//! Where termherd, Claude Code, and Antigravity keep their files, resolved
//! from an injected environment lookup.
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

/// Relocates Antigravity's data dir (`~/.gemini/antigravity-cli`), so a dev
/// instance or test can point the scanner at a fixture instead of the user's
/// real conversations.
pub const ANTIGRAVITY_DIR_VAR: &str = "TERMHERD_ANTIGRAVITY_DIR";

/// Path to the per-session private `mcpServers` config file exported into an
/// agent session's PTY environment so `termherd mcp-proxy` can forward stdio
/// MCP traffic to the parent instance's live bridge.
pub const MCP_CONFIG_VAR: &str = "TERMHERD_MCP_CONFIG";

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

/// Antigravity's data dir: [`ANTIGRAVITY_DIR_VAR`], else
/// `<home>/.gemini/antigravity-cli`.
#[must_use]
pub fn antigravity_dir(env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    overridden(env, ANTIGRAVITY_DIR_VAR)
        .or_else(|| home(env).map(|home| home.join(".gemini").join("antigravity-cli")))
}

/// Per-session live-bridge config path from [`MCP_CONFIG_VAR`], or `None` when
/// unset or empty (no fallback: an absent variable means no session bridge).
#[must_use]
pub fn mcp_config_path(env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    overridden(env, MCP_CONFIG_VAR)
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
        assert_eq!(antigravity_dir(&env_of(&[])), None);
    }

    #[test]
    fn without_overrides_all_dirs_sit_in_home() {
        let env = env_of(&[("HOME", "/h")]);
        assert_eq!(data_dir(&env), Some(PathBuf::from("/h/.termherd")));
        assert_eq!(claude_dir(&env), Some(PathBuf::from("/h/.claude")));
        assert_eq!(
            antigravity_dir(&env),
            Some(PathBuf::from("/h/.gemini/antigravity-cli"))
        );
    }

    #[test]
    fn each_override_moves_its_own_dir_only() {
        let env = env_of(&[("HOME", "/h"), (DATA_DIR_VAR, "/dev/data")]);
        assert_eq!(data_dir(&env), Some(PathBuf::from("/dev/data")));
        assert_eq!(claude_dir(&env), Some(PathBuf::from("/h/.claude")));
        assert_eq!(
            antigravity_dir(&env),
            Some(PathBuf::from("/h/.gemini/antigravity-cli"))
        );

        let env = env_of(&[("HOME", "/h"), (CLAUDE_CONFIG_DIR_VAR, "/fixture")]);
        assert_eq!(data_dir(&env), Some(PathBuf::from("/h/.termherd")));
        assert_eq!(claude_dir(&env), Some(PathBuf::from("/fixture")));
        assert_eq!(
            antigravity_dir(&env),
            Some(PathBuf::from("/h/.gemini/antigravity-cli"))
        );

        let env = env_of(&[("HOME", "/h"), (ANTIGRAVITY_DIR_VAR, "/fixture-agy")]);
        assert_eq!(data_dir(&env), Some(PathBuf::from("/h/.termherd")));
        assert_eq!(claude_dir(&env), Some(PathBuf::from("/h/.claude")));
        assert_eq!(antigravity_dir(&env), Some(PathBuf::from("/fixture-agy")));
    }

    #[test]
    fn an_override_needs_no_home() {
        let env = env_of(&[
            (DATA_DIR_VAR, "/d"),
            (CLAUDE_CONFIG_DIR_VAR, "/c"),
            (ANTIGRAVITY_DIR_VAR, "/a"),
        ]);
        assert_eq!(data_dir(&env), Some(PathBuf::from("/d")));
        assert_eq!(claude_dir(&env), Some(PathBuf::from("/c")));
        assert_eq!(antigravity_dir(&env), Some(PathBuf::from("/a")));
    }

    #[test]
    fn an_empty_override_counts_as_unset() {
        let env = env_of(&[
            ("HOME", "/h"),
            (DATA_DIR_VAR, ""),
            (CLAUDE_CONFIG_DIR_VAR, ""),
            (ANTIGRAVITY_DIR_VAR, ""),
        ]);
        assert_eq!(data_dir(&env), Some(PathBuf::from("/h/.termherd")));
        assert_eq!(claude_dir(&env), Some(PathBuf::from("/h/.claude")));
        assert_eq!(
            antigravity_dir(&env),
            Some(PathBuf::from("/h/.gemini/antigravity-cli"))
        );
    }

    #[test]
    fn mcp_config_path_reads_only_its_own_non_empty_var() {
        assert_eq!(mcp_config_path(&env_of(&[("HOME", "/h")])), None);
        assert_eq!(mcp_config_path(&env_of(&[(MCP_CONFIG_VAR, "")])), None);
        assert_eq!(
            mcp_config_path(&env_of(&[(MCP_CONFIG_VAR, "/tmp/termherd-mcp-1.json")])),
            Some(PathBuf::from("/tmp/termherd-mcp-1.json"))
        );
    }
}
