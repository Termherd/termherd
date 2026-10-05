//! Generic JSON config persistence under `~/.termherd/<file>` — the one
//! load/save shape every file adapter shares: read the file and fall back to
//! the default on any problem (missing, unreadable, corrupt — with a warning),
//! create the dir and pretty-print on save, failures logged but never fatal.
//!
//! A file that exists but does not parse is **set aside** before the default
//! is adopted, so the next save cannot overwrite what the user wrote: a
//! fallback that also destroys its input turns one bad byte into a lost
//! configuration.
//! Per-type concerns (DTO mapping, sanitising, legacy migration) stay in each
//! store; this owns only the file plumbing they used to copy.

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use tracing::warn;

/// A config file that existed and could not be used, so its defaults are in
/// force. Reported rather than only logged: a user whose settings vanished
/// deserves to be told where they went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadProblem {
    /// The file's name under `~/.termherd`, e.g. `settings.json`.
    pub file: String,
    /// Where the unparseable original was moved, or `None` when it could not
    /// be read at all (and so was left where it is).
    pub kept_as: Option<PathBuf>,
}

/// Load `~/.termherd/<file>`; any problem (no home dir, no file, bad JSON)
/// yields the default — a config file must never block startup. Touches
/// nothing on disk: safe for a read that may race a half-written file.
#[must_use]
pub fn load_json<T: Default + DeserializeOwned>(file: &str) -> T {
    let Some(path) = config_path(file) else {
        return T::default();
    };
    match read_at(&path) {
        Read::Parsed(value) => value,
        Read::Missing => T::default(),
        Read::Unreadable(e) => {
            warn!(error = %e, path = %path.display(), "unreadable config file; using defaults");
            T::default()
        }
        Read::Invalid(e) => {
            warn!(error = %e, path = %path.display(), "invalid config file; using defaults");
            T::default()
        }
    }
}

/// Re-read `~/.termherd/<file>` while the app runs: `Some` only when it parses.
/// A missing, unreadable or invalid file — an editor's half-written save
/// included — is `None`, so the caller keeps what it has rather than falling
/// back to defaults mid-run. Touches nothing on disk.
#[must_use]
pub fn reload_json<T: DeserializeOwned>(file: &str) -> Option<T> {
    reload_at(&config_path(file)?)
}

/// The body of [`reload_json`], on an explicit path.
fn reload_at<T: DeserializeOwned>(path: &Path) -> Option<T> {
    match read_at(path) {
        Read::Parsed(value) => Some(value),
        Read::Missing => {
            warn!(path = %path.display(), "config file gone; keeping the running values");
            None
        }
        Read::Unreadable(e) => {
            warn!(error = %e, path = %path.display(), "unreadable config file; keeping the running values");
            None
        }
        Read::Invalid(e) => {
            warn!(error = %e, path = %path.display(), "invalid config file; keeping the running values");
            None
        }
    }
}

/// The startup load: [`load_json`], but a file that does not parse is set
/// aside (see [`set_aside`]) and reported, so the defaults adopted in its place
/// can be saved without destroying it. Startup only — a later read may see a
/// file an editor is halfway through writing, which must not be moved.
#[must_use]
pub fn load_json_checked<T: Default + DeserializeOwned>(file: &str) -> (T, Option<LoadProblem>) {
    match config_path(file) {
        Some(path) => load_json_at(&path),
        None => (T::default(), None),
    }
}

/// The body of [`load_json_checked`], on an explicit path.
fn load_json_at<T: Default + DeserializeOwned>(path: &Path) -> (T, Option<LoadProblem>) {
    let file = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    match read_at(path) {
        Read::Parsed(value) => (value, None),
        Read::Missing => (T::default(), None),
        Read::Unreadable(e) => {
            warn!(error = %e, path = %path.display(), "unreadable config file; using defaults");
            (
                T::default(),
                Some(LoadProblem {
                    file,
                    kept_as: None,
                }),
            )
        }
        Read::Invalid(e) => {
            let kept_as = set_aside(path);
            warn!(
                error = %e,
                path = %path.display(),
                kept_as = ?kept_as,
                "invalid config file; set it aside and using defaults"
            );
            (T::default(), Some(LoadProblem { file, kept_as }))
        }
    }
}

/// What reading a config file found.
enum Read<T> {
    Parsed(T),
    /// No file: every setting starts this way, so it is no problem.
    Missing,
    Unreadable(std::io::Error),
    Invalid(serde_json::Error),
}

fn read_at<T: DeserializeOwned>(path: &Path) -> Read<T> {
    match std::fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Read::Missing,
        Err(e) => Read::Unreadable(e),
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(value) => Read::Parsed(value),
            Err(e) => Read::Invalid(e),
        },
    }
}

/// Move an unparseable file to the first free `<name>.corrupt-<n>` beside
/// it, so the next save starts a fresh file instead of overwriting the
/// original. `None` when no name is free or the move fails — then the file
/// stays, and the settings writer still refuses to write over it.
fn set_aside(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let target = (1..=99)
        .map(|n| path.with_file_name(format!("{name}.corrupt-{n}")))
        .find(|candidate| !candidate.exists())?;
    match std::fs::rename(path, &target) {
        Ok(()) => Some(target),
        Err(e) => {
            warn!(error = %e, path = %path.display(), "could not set the invalid config file aside");
            None
        }
    }
}

/// Persist `value` to `~/.termherd/<file>`. Failures are logged, never fatal —
/// losing a config write is not worth blocking the app.
pub fn save_json<T: Serialize>(file: &str, value: &T) {
    let Some(path) = config_path(file) else {
        return;
    };
    if let Err(e) = write_json_at(&path, value) {
        warn!(error = %e, "could not save config file");
    }
}

/// Pretty-print `value` to `path`, creating its directory.
fn write_json_at<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| format!("could not encode {}: {e}", path.display()))?;
    std::fs::write(path, json).map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// `~/.termherd/<file>` — the app data dir from the PRD (§7).
fn config_path(file: &str) -> Option<PathBuf> {
    Some(crate::paths::termherd_dir()?.join(file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn an_unparseable_file_is_set_aside_and_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ \"theme\": ").expect("write");

        let (value, problem) = load_json_at::<Value>(&path);

        assert_eq!(value, Value::Null, "the default is adopted");
        let kept = dir.path().join("settings.json.corrupt-1");
        assert_eq!(
            problem,
            Some(LoadProblem {
                file: "settings.json".to_string(),
                kept_as: Some(kept.clone()),
            })
        );
        assert!(!path.exists(), "a later save starts a fresh file");
        assert_eq!(
            std::fs::read_to_string(&kept).expect("kept"),
            "{ \"theme\": ",
            "the user's bytes survive verbatim"
        );
    }

    #[test]
    fn a_second_bad_file_does_not_overwrite_the_first_backup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        std::fs::write(dir.path().join("settings.json.corrupt-1"), "first").expect("write");
        std::fs::write(&path, "second").expect("write");

        let (_, problem) = load_json_at::<Value>(&path);

        assert_eq!(
            problem.and_then(|p| p.kept_as),
            Some(dir.path().join("settings.json.corrupt-2"))
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("settings.json.corrupt-1")).expect("first"),
            "first"
        );
    }

    #[test]
    fn a_reload_keeps_the_running_values_on_anything_but_a_parse_and_moves_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        assert_eq!(reload_at::<Value>(&path), None, "missing");

        std::fs::write(&path, "{ \"theme\": ").expect("write");
        assert_eq!(reload_at::<Value>(&path), None, "half-written");
        assert!(path.exists(), "a mid-run read never sets a file aside");

        std::fs::write(&path, "{\"theme\": \"light\"}").expect("write");
        assert_eq!(
            reload_at::<Value>(&path),
            Some(serde_json::json!({ "theme": "light" }))
        );
    }

    #[test]
    fn a_missing_file_is_no_problem_and_a_valid_one_loads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        assert_eq!(load_json_at::<Value>(&path), (Value::Null, None));

        std::fs::write(&path, "{\"a\": 1}").expect("write");
        assert_eq!(
            load_json_at::<Value>(&path),
            (serde_json::json!({ "a": 1 }), None)
        );
        assert!(path.exists(), "a valid file stays put");
    }
}
