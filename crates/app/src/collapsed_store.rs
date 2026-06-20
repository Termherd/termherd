//! Sidebar fold-state persistence (#22) — load/save the set of folded project
//! paths at `~/.termherd/collapsed.json`. A file adapter owned by the shell,
//! like [`crate::metadata_store`]; `core` holds the domain set and never does
//! I/O.

use std::collections::HashSet;
use std::path::PathBuf;

use tracing::warn;

/// Load the folded-project set; any problem (no file, bad JSON) yields an empty
/// set — fold state must never block startup.
#[must_use]
pub fn load() -> HashSet<String> {
    let Some(path) = config_path() else {
        return HashSet::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return HashSet::new();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        warn!(error = %e, path = %path.display(), "invalid collapsed state; ignoring");
        HashSet::new()
    })
}

/// Persist the folded-project set. Failures are logged, never fatal.
pub fn save(collapsed: &HashSet<String>) {
    let Some(path) = config_path() else {
        return;
    };
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        warn!(error = %e, "could not create config dir");
        return;
    }
    match serde_json::to_string_pretty(collapsed) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                warn!(error = %e, path = %path.display(), "could not save collapsed state");
            }
        }
        Err(e) => warn!(error = %e, "could not serialise collapsed state"),
    }
}

/// `~/.termherd/collapsed.json` — the app data dir from the PRD (§7).
fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".termherd").join("collapsed.json"))
}
