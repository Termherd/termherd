//! Startup probe for installed agent CLIs (`claude`, `codex`, `agy`).
//!
//! Runs once in `main()` and hands an [`AvailableProviders`] snapshot into the
//! shell so `core` stays pure of environment and filesystem reads.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use termherd_core::{AvailableProviders, Provider};

/// Probe the current environment's `PATH` (plus standard user binary
/// directories on Unix, where GUI launches from Finder/desktop may inherit a
/// minimal `PATH` that omits `~/.local/bin`).
#[must_use]
pub fn detect_from_env() -> AvailableProviders {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|raw| std::env::split_paths(&raw).collect())
        .unwrap_or_default();
    if !cfg!(target_os = "windows") {
        if let Some(home) = crate::paths::home_dir() {
            dirs.push(home.join(".local").join("bin"));
        }
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    let pathext = windows_pathext(std::env::var_os("PATHEXT").as_deref());
    detect_in_paths(&dirs, &pathext)
}

fn windows_pathext(raw: Option<&OsStr>) -> Vec<String> {
    if !cfg!(target_os = "windows") {
        return Vec::new();
    }
    let parsed: Vec<String> = raw
        .and_then(OsStr::to_str)
        .map(|s| {
            s.split(';')
                .map(str::trim)
                .filter(|ext| !ext.is_empty())
                .map(str::to_ascii_lowercase)
                .collect()
        })
        .unwrap_or_default();
    if parsed.is_empty() {
        vec![".exe".to_owned(), ".cmd".to_owned(), ".bat".to_owned()]
    } else {
        parsed
    }
}

fn detect_in_paths(dirs: &[PathBuf], pathext: &[String]) -> AvailableProviders {
    AvailableProviders::new(
        find_binary(Provider::Claude.binary_name(), dirs, pathext),
        find_binary(Provider::Codex.binary_name(), dirs, pathext),
        find_binary(Provider::Antigravity.binary_name(), dirs, pathext),
    )
}

fn find_binary(name: &str, dirs: &[PathBuf], pathext: &[String]) -> bool {
    dirs.iter().any(|dir| binary_in_dir(dir, name, pathext))
}

fn binary_in_dir(dir: &Path, name: &str, pathext: &[String]) -> bool {
    if dir.join(name).is_file() {
        return true;
    }
    pathext
        .iter()
        .any(|ext| dir.join(format!("{name}{ext}")).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_only_binaries_present_in_the_given_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("agy"), b"").expect("write agy");
        let found = detect_in_paths(&[dir.path().to_path_buf()], &[]);
        assert!(!found.is_available(Provider::Claude));
        assert!(!found.is_available(Provider::Codex));
        assert!(found.is_available(Provider::Antigravity));
    }

    #[test]
    fn matches_pathext_suffixes_when_supplied() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("agy.exe"), b"").expect("write agy.exe");
        std::fs::write(dir.path().join("claude.cmd"), b"").expect("write claude.cmd");
        let exts = vec![".exe".to_owned(), ".cmd".to_owned()];
        let found = detect_in_paths(&[dir.path().to_path_buf()], &exts);
        assert!(found.is_available(Provider::Claude));
        assert!(!found.is_available(Provider::Codex));
        assert!(found.is_available(Provider::Antigravity));
    }

    #[test]
    fn ignores_directories_sharing_a_binary_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("agy")).expect("mkdir agy");
        let found = detect_in_paths(&[dir.path().to_path_buf()], &[]);
        assert!(!found.is_available(Provider::Antigravity));
    }
}
