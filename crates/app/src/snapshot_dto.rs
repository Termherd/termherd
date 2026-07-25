//! The JSON wire form of [`WorkspaceSnapshot`] — `core`'s model flattened to
//! string-typed enums a reader consumes without knowing termherd's internals.
//!
//! It lives in the `app` adapter, not in `core`: `core` carries no serde
//! dependency, so the snapshot stays a plain value and the adapter owns its wire
//! form. Absent sections and empty terminal text are omitted, keeping a light
//! read light.

use std::collections::BTreeMap;

use serde::Serialize;
use termherd_core::{SessionStatus, WorkspaceSnapshot};

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
        kind: match pane.kind {
            termherd_core::SessionKind::Shell => "shell",
            termherd_core::SessionKind::Claude => "claude",
        },
        cwd: pane.cwd.clone(),
        status: status_str(pane.status),
    }
}
