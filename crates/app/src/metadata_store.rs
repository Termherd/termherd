//! Metadata persistence (`F-session-metadata`, `F-favorites`) — load/save the
//! whole user overlay at `~/.termherd/metadata.json`. A file adapter owned by
//! the shell, like [`crate::settings`]; `core` holds the domain [`Overlay`]
//! (sessions + repos) and never does I/O.
//!
//! The on-disk shape is `{ "sessions": {…}, "repos": {…} }`. Older builds wrote
//! a **flat** map of session id → meta with no wrapper; [`load`] migrates such
//! files in place (see [`parse`]), and [`save`] always writes the new shape.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use termherd_core::{Overlay, RepoMeta, SessionMeta};
use tracing::warn;

/// On-disk shape of one session entry. Default fields are skipped so the file
/// only records what the user actually set.
#[derive(Debug, Default, Serialize, Deserialize)]
struct MetaDto {
    #[serde(default, skip_serializing_if = "is_false")]
    starred: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    archived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
}

/// On-disk shape of one repo entry.
#[derive(Debug, Default, Serialize, Deserialize)]
struct RepoDto {
    #[serde(default, skip_serializing_if = "is_false")]
    starred: bool,
}

/// On-disk shape of the whole file: the two keyings under named wrappers.
#[derive(Debug, Default, Serialize, Deserialize)]
struct OverlayDto {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    sessions: HashMap<String, MetaDto>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    repos: HashMap<String, RepoDto>,
}

fn is_false(b: &bool) -> bool {
    !b
}

/// Load the overlay; any problem (no file, bad JSON) yields an empty overlay —
/// metadata must never block startup.
#[must_use]
pub fn load() -> Overlay {
    let Some(path) = config_path() else {
        return Overlay::default();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Overlay::default();
    };
    parse(&raw).unwrap_or_else(|e| {
        warn!(error = %e, path = %path.display(), "invalid metadata; ignoring");
        Overlay::default()
    })
}

/// Parse raw JSON into an [`Overlay`], migrating the legacy flat shape.
///
/// A new-format file has a top-level `sessions` or `repos` key; anything else is
/// read as the legacy flat `{ id: meta }` map (session ids are Claude UUIDs, so
/// they never collide with those two literal wrapper keys). Kept separate from
/// [`load`] so the migration is unit-tested without touching the filesystem.
fn parse(raw: &str) -> Result<Overlay, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_str(raw)?;
    let is_wrapped = value.get("sessions").is_some() || value.get("repos").is_some();
    let dto: OverlayDto = if is_wrapped {
        serde_json::from_value(value)?
    } else {
        OverlayDto {
            sessions: serde_json::from_value(value)?,
            repos: HashMap::new(),
        }
    };
    Ok(from_dto(dto))
}

/// Persist the overlay. Failures are logged, never fatal.
pub fn save(overlay: &Overlay) {
    let Some(path) = config_path() else {
        return;
    };
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        warn!(error = %e, "could not create config dir");
        return;
    }
    let dto = to_dto(overlay);
    match serde_json::to_string_pretty(&dto) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                warn!(error = %e, path = %path.display(), "could not save metadata");
            }
        }
        Err(e) => warn!(error = %e, "could not serialise metadata"),
    }
}

fn from_dto(dto: OverlayDto) -> Overlay {
    Overlay {
        sessions: dto
            .sessions
            .into_iter()
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
            .collect(),
        repos: dto
            .repos
            .into_iter()
            .map(|(path, m)| (path, RepoMeta { starred: m.starred }))
            .collect(),
    }
}

fn to_dto(overlay: &Overlay) -> OverlayDto {
    OverlayDto {
        sessions: overlay
            .sessions
            .iter()
            .map(|(id, m)| {
                (
                    id.clone(),
                    MetaDto {
                        starred: m.starred,
                        archived: m.archived,
                        title: m.title.clone(),
                    },
                )
            })
            .collect(),
        repos: overlay
            .repos
            .iter()
            .map(|(path, m)| (path.clone(), RepoDto { starred: m.starred }))
            .collect(),
    }
}

/// `~/.termherd/metadata.json` — the app data dir from the PRD (§7).
fn config_path() -> Option<PathBuf> {
    Some(crate::paths::termherd_dir()?.join("metadata.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_flat_file_migrates_to_sessions() {
        // Pre-`F-favorites` files were a bare `{ id: meta }` map, no wrapper.
        let raw = r#"{ "abc-123": { "starred": true, "title": "Ship it" } }"#;
        let overlay = parse(raw).unwrap();
        assert!(overlay.sessions["abc-123"].starred);
        assert_eq!(
            overlay.sessions["abc-123"].title.as_deref(),
            Some("Ship it")
        );
        assert!(overlay.repos.is_empty());
    }

    #[test]
    fn new_wrapped_file_loads_both_maps() {
        let raw = r#"{
            "sessions": { "abc-123": { "archived": true } },
            "repos": { "/home/me/dev/termherd": { "starred": true } }
        }"#;
        let overlay = parse(raw).unwrap();
        assert!(overlay.sessions["abc-123"].archived);
        assert!(overlay.repos["/home/me/dev/termherd"].starred);
    }

    #[test]
    fn a_repos_only_file_is_recognised_as_new_format() {
        // Only `repos` present — must not be mistaken for a legacy flat map.
        let raw = r#"{ "repos": { "/p": { "starred": true } } }"#;
        let overlay = parse(raw).unwrap();
        assert!(overlay.repos["/p"].starred);
        assert!(overlay.sessions.is_empty());
    }

    #[test]
    fn save_then_parse_round_trips() {
        let mut overlay = Overlay::default();
        overlay.sessions.insert(
            "s1".into(),
            SessionMeta {
                starred: true,
                ..Default::default()
            },
        );
        overlay
            .repos
            .insert("/p".into(), RepoMeta { starred: true });
        let json = serde_json::to_string(&to_dto(&overlay)).unwrap();
        let back = parse(&json).unwrap();
        assert!(back.sessions["s1"].starred);
        assert!(back.repos["/p"].starred);
    }

    #[test]
    fn empty_object_is_an_empty_overlay() {
        let overlay = parse("{}").unwrap();
        assert!(overlay.sessions.is_empty() && overlay.repos.is_empty());
    }
}
