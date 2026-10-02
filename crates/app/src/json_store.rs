//! Generic JSON config persistence under `~/.termherd/<file>` — the one
//! load/save shape every file adapter shares: read the file and fall back to
//! the default on any problem (missing, unreadable, corrupt — with a warning),
//! create the dir and pretty-print on save, failures logged but never fatal.
//! Per-type concerns (DTO mapping, sanitising, legacy migration) stay in each
//! store; this owns only the file plumbing they used to copy.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::path::PathBuf;
use tracing::warn;

/// Load `~/.termherd/<file>`; any problem (no home dir, no file, bad JSON)
/// yields the default — a config file must never block startup.
#[must_use]
pub fn load_json<T: Default + DeserializeOwned>(file: &str) -> T {
    let Some(path) = config_path(file) else {
        return T::default();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return T::default();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        warn!(error = %e, path = %path.display(), "invalid config file; using defaults");
        T::default()
    })
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
