//! Session-browser domain — scan results grouped into sidebar projects.
//! Pure data + pure grouping; the scan adapter produces [`SessionRecord`]s.
//!
//! FR1: one group per distinct real project path — the duplicate-sidebar
//! bug class (#41/#44) is pinned here by construction and by tests.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

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

/// A compact, language-neutral relative age — `now`, `5m`, `3h`, `2d`, `4w`,
/// `1y` — used to disambiguate sidebar rows whose titles collide within a
/// project (#42). The caller supplies the elapsed `Duration`: core stays pure
/// (no clock), the adapter owns the wall clock.
#[must_use]
pub fn relative_age(elapsed: Duration) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    const YEAR: u64 = 365 * DAY;

    let secs = elapsed.as_secs();
    if secs < MINUTE {
        "now".to_owned()
    } else if secs < HOUR {
        format!("{}m", secs / MINUTE)
    } else if secs < DAY {
        format!("{}h", secs / HOUR)
    } else if secs < WEEK {
        format!("{}d", secs / DAY)
    } else if secs < YEAR {
        format!("{}w", secs / WEEK)
    } else {
        format!("{}y", secs / YEAR)
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

/// Filter groups for the search box (FR3): case-insensitive, matching the
/// project path or, per session, the display title — plus summary, slug and
/// indexed text unless `titles_only`. Groups keep their order; a group
/// whose path matches is kept whole.
#[must_use]
pub fn filter_projects(
    groups: &[ProjectGroup],
    query: &str,
    titles_only: bool,
) -> Vec<ProjectGroup> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return groups.to_vec();
    }
    groups
        .iter()
        .filter_map(|group| {
            if group.path.to_lowercase().contains(&needle) {
                return Some(group.clone());
            }
            let sessions: Vec<SessionRecord> = group
                .sessions
                .iter()
                .filter(|s| session_matches(s, &needle, titles_only))
                .cloned()
                .collect();
            if sessions.is_empty() {
                None
            } else {
                Some(ProjectGroup {
                    path: group.path.clone(),
                    sessions,
                })
            }
        })
        .collect()
}

fn session_matches(session: &SessionRecord, needle_lower: &str, titles_only: bool) -> bool {
    if session
        .digest
        .display_title(None)
        .to_lowercase()
        .contains(needle_lower)
    {
        return true;
    }
    if titles_only {
        return false;
    }
    let d = &session.digest;
    d.summary.to_lowercase().contains(needle_lower)
        || d.slug
            .as_deref()
            .is_some_and(|s| s.to_lowercase().contains(needle_lower))
        || d.text_content.to_lowercase().contains(needle_lower)
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
                tail: Vec::new(),
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

    #[test]
    fn empty_query_keeps_everything() {
        let groups = group_projects(vec![record("/a", "s1", 1)]);
        assert_eq!(filter_projects(&groups, "  ", false), groups);
    }

    #[test]
    fn search_is_case_insensitive_over_content_and_titles() {
        let mut r = record("/a", "s1", 1);
        r.digest.text_content = "Fix the Login Bug\n".into();
        let groups = group_projects(vec![r, record("/b", "s2", 2)]);

        let hits = filter_projects(&groups, "lOgIn", false);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "/a");
        // Content does not match in titles-only mode…
        assert!(filter_projects(&groups, "login", true).is_empty());
        // …but titles still do ("prompt s2" is the display title).
        let hits = filter_projects(&groups, "PROMPT S2", true);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "/b");
    }

    #[test]
    fn matching_project_path_keeps_the_whole_group() {
        let groups = group_projects(vec![
            record("/apps/web", "s1", 1),
            record("/apps/web", "s2", 2),
        ]);
        let hits = filter_projects(&groups, "web", true);
        assert_eq!(hits[0].sessions.len(), 2);
    }

    #[test]
    fn relative_age_picks_the_largest_fitting_unit() {
        assert_eq!(relative_age(Duration::from_secs(0)), "now");
        assert_eq!(relative_age(Duration::from_secs(59)), "now");
        assert_eq!(relative_age(Duration::from_secs(60)), "1m");
        assert_eq!(relative_age(Duration::from_secs(59 * 60)), "59m");
        assert_eq!(relative_age(Duration::from_secs(3600)), "1h");
        assert_eq!(relative_age(Duration::from_secs(25 * 3600)), "1d");
        assert_eq!(relative_age(Duration::from_secs(8 * 86_400)), "1w");
        assert_eq!(relative_age(Duration::from_secs(400 * 86_400)), "1y");
    }

    #[test]
    fn non_matching_sessions_are_dropped_from_a_group() {
        let mut hit = record("/a", "findme", 1);
        hit.digest.summary = "the needle".into();
        let groups = group_projects(vec![hit, record("/a", "other", 2)]);
        let filtered = filter_projects(&groups, "needle", false);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].sessions.len(), 1);
        assert_eq!(filtered[0].sessions[0].session_id, "findme");
    }
}
