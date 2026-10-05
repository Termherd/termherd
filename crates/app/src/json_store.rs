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
use serde_json::{Map, Value};
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
/// stays, and [`update_json`] still refuses to write over it.
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
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        warn!(error = %e, "could not create config dir");
        return;
    }
    match serde_json::to_string_pretty(value) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                warn!(error = %e, path = %path.display(), "could not save config file");
            }
        }
        Err(e) => warn!(error = %e, path = %path.display(), "could not serialise config"),
    }
}

/// Rewrite some keys of the JSON object in `~/.termherd/<file>`, leaving every
/// other key as found — including ones this build does not understand. A
/// missing file starts from an empty object; a file that does not parse as an
/// object is left alone (with a warning), since rewriting it would replace the
/// user's whole configuration with the few keys being set.
pub fn update_json(file: &str, edit: impl FnOnce(&mut Map<String, Value>)) {
    let Some(path) = config_path(file) else {
        return;
    };
    let mut root = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => {
            warn!(error = %e, path = %path.display(), "could not read config file; not updating it");
            return;
        }
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(Value::Object(root)) => root,
            _ => {
                warn!(path = %path.display(), "config file is not a JSON object; not updating it");
                return;
            }
        },
    };
    edit(&mut root);
    save_json(file, &root);
}

/// `~/.termherd/<file>` — the app data dir from the PRD (§7).
fn config_path(file: &str) -> Option<PathBuf> {
    Some(crate::paths::termherd_dir()?.join(file))
}

#[cfg(test)]
mod tests {
    use super::*;

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
