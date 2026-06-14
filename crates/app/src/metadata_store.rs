//! Session-metadata persistence (`F-session-metadata`) — load/save the star /
//! archive / title overlay at `~/.termherd/metadata.json`. A file adapter
//! owned by the shell, like [`crate::settings`]; `core` holds the domain
//! [`SessionMeta`] and never does I/O.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use termherd_core::SessionMeta;
use tracing::warn;

/// On-disk shape of one entry. Default fields are skipped so the file only
/// records what the user actually set.
#[derive(Debug, Default, Serialize, Deserialize)]
struct MetaDto {
    #[serde(default, skip_serializing_if = "is_false")]
    starred: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    archived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !b
}

/// Load the overlay; any problem (no file, bad JSON) yields an empty map —
/// metadata must never block startup.
#[must_use]
pub fn load() -> HashMap<String, SessionMeta> {
    let Some(path) = config_path() else {
        return HashMap::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    let dto: HashMap<String, MetaDto> = serde_json::from_str(&raw).unwrap_or_else(|e| {
        warn!(error = %e, path = %path.display(), "invalid metadata; ignoring");
        HashMap::new()
    });
    dto.into_iter()
        .map(|(id, m)| {
            (
                id,
                SessionMeta {
                    starred: m.starred,
                    archived: m.archived,
                    title: m.title,
                },
            )
        })
        .collect()
}

/// Persist the overlay. Failures are logged, never fatal.
pub fn save(metadata: &HashMap<String, SessionMeta>) {
    let Some(path) = config_path() else {
        return;
    };
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        warn!(error = %e, "could not create config dir");
        return;
    }
    let dto: HashMap<&str, MetaDto> = metadata
        .iter()
        .map(|(id, m)| {
            (
                id.as_str(),
                MetaDto {
                    starred: m.starred,
                    archived: m.archived,
                    title: m.title.clone(),
                },
            )
        })
        .collect();
    match serde_json::to_string_pretty(&dto) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                warn!(error = %e, path = %path.display(), "could not save metadata");
            }
        }
        Err(e) => warn!(error = %e, "could not serialise metadata"),
    }
}

/// `~/.termherd/metadata.json` — the app data dir from the PRD (§7).
fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".termherd").join("metadata.json"))
}
