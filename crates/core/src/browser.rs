//! Session-browser domain — scan results grouped into sidebar projects.
//! Pure data + pure grouping; the scan adapter produces [`SessionRecord`]s.
//!
//! FR1: one group per distinct real project path — the duplicate-sidebar
//! bug class (#41/#44) is pinned here by construction and by tests.

use std::collections::BTreeMap;
use std::time::SystemTime;

use termherd_claude::digest::SessionDigest;

/// One discovered session, as delivered by the scan adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRecord {
    /// JSONL file stem — Claude CLI's session UUID.
    pub session_id: String,
    /// Real project path: derived from `cwd` and worktree-collapsed (with
    /// the fs existence check) by the adapter before it reaches the core.
    pub project_path: String,
    pub digest: SessionDigest,
    /// File mtime; `None` when the filesystem could not provide one.
    pub modified: Option<SystemTime>,
}

/// One sidebar project: its sessions, most recent first.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectGroup {
    pub path: String,
    pub sessions: Vec<SessionRecord>,
}

impl ProjectGroup {
    /// When this project last saw activity (its freshest session).
    #[must_use]
    pub fn last_activity(&self) -> Option<SystemTime> {
        self.sessions.iter().filter_map(|s| s.modified).max()
    }
}

/// Group records by project path. Sessions are sorted most-recent-first
/// inside each group; groups are sorted by most recent activity (sessions
/// without an mtime sort last, ties broken by path for determinism).
#[must_use]
pub fn group_projects(records: Vec<SessionRecord>) -> Vec<ProjectGroup> {
    let mut by_path: BTreeMap<String, Vec<SessionRecord>> = BTreeMap::new();
    for record in records {
        by_path
            .entry(record.project_path.clone())
            .or_default()
            .push(record);
    }
    let mut groups: Vec<ProjectGroup> = by_path
        .into_iter()
        .map(|(path, mut sessions)| {
            sessions.sort_by_key(|s| std::cmp::Reverse(s.modified));
            ProjectGroup { path, sessions }
        })
        .collect();
    // BTreeMap iteration gives path order; the stable sort keeps it as the
    // tie-breaker for equal activity.
    groups.sort_by_key(|g| std::cmp::Reverse(g.last_activity()));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn record(project: &str, id: &str, age_secs: u64) -> SessionRecord {
        SessionRecord {
            session_id: id.to_owned(),
            project_path: project.to_owned(),
            digest: SessionDigest {
                summary: format!("prompt {id}"),
                message_count: 1,
                text_content: String::new(),
                slug: None,
                custom_title: None,
                ai_title: None,
            },
            modified: Some(UNIX_EPOCH + Duration::from_secs(age_secs)),
        }
    }

    #[test]
    fn one_group_per_distinct_path() {
        let groups = group_projects(vec![
            record("/a", "s1", 10),
            record("/b", "s2", 20),
            record("/a", "s3", 30),
        ]);
        assert_eq!(groups.len(), 2);
        let a = groups.iter().find(|g| g.path == "/a").unwrap();
        assert_eq!(a.sessions.len(), 2);
    }

    #[test]
    fn sessions_are_most_recent_first() {
        let groups = group_projects(vec![record("/a", "old", 10), record("/a", "new", 99)]);
        let ids: Vec<_> = groups[0]
            .sessions
            .iter()
            .map(|s| s.session_id.as_str())
            .collect();
        assert_eq!(ids, vec!["new", "old"]);
    }

    #[test]
    fn groups_are_ordered_by_latest_activity() {
        let groups = group_projects(vec![
            record("/quiet", "s1", 10),
            record("/busy", "s2", 100),
            record("/quiet", "s3", 50),
        ]);
        let paths: Vec<_> = groups.iter().map(|g| g.path.as_str()).collect();
        assert_eq!(paths, vec!["/busy", "/quiet"]);
    }

    #[test]
    fn missing_mtimes_sort_last_and_ties_break_by_path() {
        let mut no_mtime = record("/z", "s1", 0);
        no_mtime.modified = None;
        let groups = group_projects(vec![no_mtime, record("/b", "s2", 5), record("/a", "s3", 5)]);
        let paths: Vec<_> = groups.iter().map(|g| g.path.as_str()).collect();
        assert_eq!(paths, vec!["/a", "/b", "/z"]);
    }

    #[test]
    fn empty_input_gives_empty_sidebar() {
        assert!(group_projects(vec![]).is_empty());
    }
}
