//! The JSON wire form of [`WorkspaceSnapshot`] — `core`'s model flattened to
//! string-typed enums a reader consumes without knowing termherd's internals.
//!
//! It lives in the `app` adapter, not in `core`: `core` carries no serde
//! dependency, so the snapshot stays a plain value and the adapter owns its wire
//! form. Absent sections and empty terminal text are omitted, keeping a light
//! read light.

use std::collections::BTreeMap;

use serde::Serialize;
use termherd_core::{ClaudeColor, ClaudeIdentity, SessionKind, SessionStatus, WorkspaceSnapshot};

/// The on-the-wire snapshot.
#[derive(Serialize)]
pub(crate) struct SnapshotDto {
    focus: FocusDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    config: Option<ConfigDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sidebar: Option<SidebarDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tabs: Option<Vec<TabDto>>,
    /// Scoped terminal text by handle (string keys in JSON). Empty when none was
    /// requested.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    terminals: BTreeMap<u64, String>,
}

#[derive(Serialize)]
struct FocusDto {
    tab: Option<usize>,
    /// Focused session handle as a string, matching `list_sessions`.
    session: Option<String>,
}

#[derive(Serialize)]
struct ConfigDto {
    font_size: f32,
    terminal_scheme: Option<String>,
    record_fps: u32,
    record_scale: f32,
    keymap_overrides: usize,
}

#[derive(Serialize)]
struct SidebarDto {
    hidden: bool,
    search: String,
    search_titles_only: bool,
    show_archived: bool,
    projects: Vec<ProjectDto>,
}

#[derive(Serialize)]
struct ProjectDto {
    path: String,
    session_count: usize,
    collapsed: bool,
    declared: bool,
}

#[derive(Serialize)]
struct TabDto {
    active: bool,
    title: String,
    /// Most-urgent status among the tab's sessions, or `None` if none live.
    status: Option<&'static str>,
    panes: Vec<PaneDto>,
}

#[derive(Serialize)]
struct PaneDto {
    /// Stable session handle as a string, matching `list_sessions` and the
    /// `terminals` argument.
    handle: String,
    /// `"shell"` or `"claude"`.
    kind: &'static str,
    cwd: Option<String>,
    /// `"starting"`, `"busy"`, `"idle"`, `"attention"`, or `"exited"`.
    status: &'static str,
    #[serde(flatten)]
    identity: IdentityDto,
    /// The colour the pane wears — Claude's `/color` for a Claude pane, the
    /// tab's picked colour for a shell — by its `/color` name; `null` when it
    /// wears none.
    color: Option<&'static str>,
}

/// Who the Claude in a pane is, as three flat fields shared by a snapshot pane
/// and a `list_sessions` row. Each is `null`, never omitted, when unknown: no
/// Claude in front, Windows, or a Claude Code that wrote no session file.
#[derive(Serialize)]
pub(crate) struct IdentityDto {
    pid: Option<u32>,
    peer_name: Option<String>,
    session_id: Option<String>,
}

impl From<&ClaudeIdentity> for IdentityDto {
    fn from(identity: &ClaudeIdentity) -> Self {
        Self {
            pid: identity.pid,
            peer_name: identity.peer_name.clone(),
            session_id: identity.session_id.clone(),
        }
    }
}

/// The stable external string for a session kind — one place every DTO reads.
pub(crate) fn kind_str(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Shell => "shell",
        SessionKind::Claude => "claude",
    }
}

/// The stable external string for a session status — one place every DTO reads.
pub(crate) fn status_str(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Starting => "starting",
        SessionStatus::Busy => "busy",
        SessionStatus::Idle => "idle",
        SessionStatus::Attention => "attention",
        SessionStatus::Exited => "exited",
    }
}

impl From<&WorkspaceSnapshot> for SnapshotDto {
    fn from(snapshot: &WorkspaceSnapshot) -> Self {
        Self {
            focus: FocusDto {
                tab: snapshot.focus.tab,
                session: snapshot.focus.session.map(|handle| handle.to_string()),
            },
            config: snapshot.config.as_ref().map(|config| ConfigDto {
                font_size: config.font_size,
                terminal_scheme: config.terminal_scheme.clone(),
                record_fps: config.record_fps,
                record_scale: config.record_scale,
                keymap_overrides: config.keymap_overrides,
            }),
            sidebar: snapshot.sidebar.as_ref().map(|sidebar| SidebarDto {
                hidden: sidebar.hidden,
                search: sidebar.search.clone(),
                search_titles_only: sidebar.search_titles_only,
                show_archived: sidebar.show_archived,
                projects: sidebar
                    .projects
                    .iter()
                    .map(|project| ProjectDto {
                        path: project.path.clone(),
                        session_count: project.session_count,
                        collapsed: project.collapsed,
                        declared: project.declared,
                    })
                    .collect(),
            }),
            tabs: snapshot.tabs.as_ref().map(|tabs| {
                tabs.iter()
                    .map(|tab| TabDto {
                        active: tab.active,
                        title: tab.title.clone(),
                        status: tab.status.map(status_str),
                        panes: tab.panes.iter().map(pane_dto).collect(),
                    })
                    .collect()
            }),
            terminals: snapshot.terminals.clone(),
        }
    }
}

/// One pane on the wire. Free function — it reads only the pane.
fn pane_dto(pane: &termherd_core::PaneSnapshot) -> PaneDto {
    PaneDto {
        handle: pane.handle.to_string(),
        kind: kind_str(pane.kind),
        cwd: pane.cwd.clone(),
        status: status_str(pane.status),
        identity: IdentityDto::from(&pane.identity),
        color: pane.color.map(ClaudeColor::name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use termherd_core::PaneSnapshot;

    fn pane(kind: SessionKind, identity: ClaudeIdentity) -> serde_json::Value {
        coloured_pane(kind, identity, None)
    }

    fn coloured_pane(
        kind: SessionKind,
        identity: ClaudeIdentity,
        color: Option<ClaudeColor>,
    ) -> serde_json::Value {
        serde_json::to_value(pane_dto(&PaneSnapshot {
            handle: 7,
            kind,
            cwd: Some("/proj".to_owned()),
            status: SessionStatus::Idle,
            identity,
            color,
        }))
        .expect("encode")
    }

    #[test]
    fn a_coloured_pane_names_its_colour_as_slash_color_spells_it() {
        let json = coloured_pane(
            SessionKind::Claude,
            ClaudeIdentity::default(),
            Some(ClaudeColor::Purple),
        );
        assert_eq!(json["color"], "purple");
    }

    #[test]
    fn a_claude_pane_carries_pid_peer_name_and_session_id_flat() {
        let json = pane(
            SessionKind::Claude,
            ClaudeIdentity {
                pid: Some(4399),
                peer_name: Some("proj-35".to_owned()),
                session_id: Some("7eff".to_owned()),
            },
        );
        assert_eq!(json["pid"], 4399);
        assert_eq!(json["peer_name"], "proj-35");
        assert_eq!(json["session_id"], "7eff");
    }

    #[test]
    fn an_unknown_identity_is_null_not_missing() {
        // A reader tells "termherd does not know" from "this termherd predates
        // the field" by the key being there.
        let json = pane(SessionKind::Shell, ClaudeIdentity::default());
        for key in ["pid", "peer_name", "session_id", "color"] {
            assert_eq!(
                json.get(key),
                Some(&serde_json::Value::Null),
                "{key} must be present and null"
            );
        }
    }
}
