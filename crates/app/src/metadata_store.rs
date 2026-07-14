//! Metadata persistence (`F-session-metadata`, `F-favorites`) — load/save the
//! whole user overlay at `~/.termherd/metadata.json`. A file adapter owned by
//! the shell, like [`crate::settings`]; `core` holds the domain [`Overlay`]
//! (sessions + repos) and never does I/O.
//!
//! The on-disk shape is `{ "sessions": {…}, "repos": {…} }`. Older builds wrote
//! a **flat** map of session id → meta with no wrapper; [`StoredOverlay`]'s
//! deserialiser migrates such files in place, and [`save`] always writes the
//! new shape. The file plumbing lives in [`crate::json_store`].

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use termherd_core::{Overlay, RepoMeta, SessionMeta};

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

/// The on-disk file as loaded, with the legacy migration applied: a new-format
/// file has a top-level `sessions` or `repos` key; anything else is read as
/// the legacy flat `{ id: meta }` map (session ids are Claude UUIDs, so they
/// never collide with those two literal wrapper keys). A custom deserialiser
/// so the migration rides any parse of the file, wherever it comes from.
#[derive(Debug, Default)]
struct StoredOverlay(OverlayDto);

impl<'de> Deserialize<'de> for StoredOverlay {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = serde_json::Value::deserialize(deserializer)?;
        let is_wrapped = value.get("sessions").is_some() || value.get("repos").is_some();
        let dto = if is_wrapped {
            serde_json::from_value(value)
        } else {
            serde_json::from_value(value).map(|sessions| OverlayDto {
                sessions,
                repos: HashMap::new(),
            })
        }
        .map_err(D::Error::custom)?;
        Ok(Self(dto))
    }
}

/// Load the overlay; any problem (no file, bad JSON) yields an empty overlay —
/// metadata must never block startup.
#[must_use]
pub fn load() -> Overlay {
    from_dto(crate::json_store::load_json::<StoredOverlay>(FILE).0)
}

/// Persist the overlay. Failures are logged, never fatal.
pub fn save(overlay: &Overlay) {
    crate::json_store::save_json(FILE, &to_dto(overlay));
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
const FILE: &str = "metadata.json";

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse raw JSON as [`load`] would, exercising the legacy migration
    /// without touching the filesystem.
    fn parse(raw: &str) -> Result<Overlay, serde_json::Error> {
        serde_json::from_str::<StoredOverlay>(raw).map(|stored| from_dto(stored.0))
    }

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
