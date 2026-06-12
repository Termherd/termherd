//! Headless `App` — pure state machine over `Event`/`Effect`.
//!
//! The quality keystone (see `docs/ARCHITECTURE.md` §5). Events and effects
//! grow incrementally with each milestone.

use crate::browser::{ProjectGroup, SessionRecord, group_projects};
use crate::workspace::Workspace;

#[derive(Debug, Default)]
pub struct App {
    pub workspace: Workspace,
    /// Sidebar state: projects grouped from the latest scan (FR1).
    pub projects: Vec<ProjectGroup>,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// A filesystem scan finished; replaces the whole browser state
    /// (incremental updates come with the `notify` watcher).
    ScanCompleted(Vec<SessionRecord>),
}

/// Side effects the runtime must perform. None exist yet — spawn/resize
/// land in M2 — so this is deliberately uninhabited for now.
#[derive(Debug, Clone)]
pub enum Effect {}

impl App {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply an event, returning the effects the runtime must carry out.
    /// **Pure**: no I/O, no clock, no panic.
    pub fn apply(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::ScanCompleted(records) => {
                self.projects = group_projects(records);
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use termherd_claude::digest::SessionDigest;

    #[test]
    fn scan_completed_rebuilds_projects_and_yields_no_effects() {
        let mut app = App::new();
        let record = SessionRecord {
            session_id: "abc".into(),
            project_path: "/p".into(),
            digest: SessionDigest {
                summary: "hello".into(),
                message_count: 1,
                text_content: String::new(),
                slug: None,
                custom_title: None,
                ai_title: None,
            },
            modified: None,
        };
        let effects = app.apply(Event::ScanCompleted(vec![record]));
        assert!(effects.is_empty());
        assert_eq!(app.projects.len(), 1);
        assert_eq!(app.projects[0].path, "/p");

        // A later scan replaces, not appends.
        let effects = app.apply(Event::ScanCompleted(vec![]));
        assert!(effects.is_empty());
        assert!(app.projects.is_empty());
    }
}
