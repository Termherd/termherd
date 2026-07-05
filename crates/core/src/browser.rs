//! Session-browser domain — scan results grouped into sidebar projects.
//! Pure data + pure grouping; the scan adapter produces [`SessionRecord`]s.
//!
//! FR1: one group per distinct real project path — the duplicate-sidebar
//! bug class is pinned here by construction and by tests.

use std::collections::BTreeMap;
use std::ops::Range;
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
/// project. The caller supplies the elapsed `Duration`: core stays pure
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

/// Name a project from its path: the last non-empty path component, treating
/// both `/` and `\` as separators so Windows and collapsed-worktree paths land
/// on the same rule. Falls back to the whole input when it is all separators or
/// empty, so the label is never blank.
#[must_use]
pub fn project_label(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path)
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
    content_snippet(&session.digest, needle_lower).is_some()
}

/// A located content match: the line that matched, truncated around the
/// hit when long, plus the byte range within [`Self::line`] the needle covers.
/// Shown in muted text under a session row so a content hit reveals *what*
/// matched, not merely *that* something did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchSnippet {
    /// The matched line, with `…` marking either end that was clipped.
    pub line: String,
    /// Byte range of the match within [`Self::line`] (after truncation).
    pub range: Range<usize>,
}

/// Most chars [`MatchSnippet::line`] shows; longer matched lines are windowed
/// around the hit.
const SNIPPET_MAX_CHARS: usize = 80;
/// How many chars of leading context to keep before the hit when windowing.
const SNIPPET_LEAD_CHARS: usize = 16;
const ELLIPSIS: char = '…';

/// Locate the first content hit for an already-lowercased `needle_lower` across
/// a session's summary, indexed text (line by line), and slug — the same
/// fields [`session_matches`] tests, so a content match always yields a
/// snippet. `None` when nothing but the title matched (or nothing did).
#[must_use]
pub fn content_snippet(digest: &SessionDigest, needle_lower: &str) -> Option<MatchSnippet> {
    if needle_lower.is_empty() {
        return None;
    }
    std::iter::once(digest.summary.as_str())
        .chain(digest.text_content.lines())
        .chain(digest.slug.as_deref())
        .find_map(|line| locate(line, needle_lower).map(|hit| snippet_around(line, hit)))
}

/// Byte range of the first case-insensitive occurrence of `needle_lower` in
/// `line`. Lowercasing can shift byte offsets (rare, non-ASCII case-folding);
/// when the mapped offsets no longer land on char boundaries we fall back to
/// flagging the whole line rather than risk slicing mid-character.
fn locate(line: &str, needle_lower: &str) -> Option<Range<usize>> {
    let start = line.to_lowercase().find(needle_lower)?;
    let end = start + needle_lower.len();
    if line.is_char_boundary(start) && line.is_char_boundary(end) {
        Some(start..end)
    } else {
        Some(0..line.len())
    }
}

/// Window `line` to at most [`SNIPPET_MAX_CHARS`] around `hit`, keeping a little
/// leading context and marking each clipped end with `…`. The returned range is
/// re-based onto the (possibly shortened, `…`-prefixed) output.
fn snippet_around(line: &str, hit: Range<usize>) -> MatchSnippet {
    // Every valid slice point, in order — the hit endpoints are among them.
    let bounds: Vec<usize> = line
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(line.len()))
        .collect();
    let last = bounds.len().saturating_sub(1);
    let si = bounds.iter().position(|&b| b == hit.start).unwrap_or(0);
    let ei = bounds.iter().position(|&b| b == hit.end).unwrap_or(last);

    let ws = si.saturating_sub(SNIPPET_LEAD_CHARS);
    let we = (ws + SNIPPET_MAX_CHARS).max(ei).min(last);

    let mut out = String::new();
    let mut range_start = hit.start - bounds[ws];
    if ws > 0 {
        out.push(ELLIPSIS);
        range_start += ELLIPSIS.len_utf8();
    }
    out.push_str(&line[bounds[ws]..bounds[we]]);
    if we < last {
        out.push(ELLIPSIS);
    }
    let range_end = range_start + (hit.end - hit.start);
    MatchSnippet {
        line: out,
        range: range_start..range_end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
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

    #[test]
    fn content_snippet_locates_the_match_in_the_summary() {
        let mut r = record("/a", "s1", 1);
        r.digest.summary = "Fix the login bug".into();
        let snip = content_snippet(&r.digest, "login").unwrap();
        assert_eq!(snip.line, "Fix the login bug");
        // The span carries original case; the needle is lowercased only to find.
        assert_eq!(&snip.line[snip.range.clone()], "login");
    }

    #[test]
    fn content_snippet_finds_a_hit_inside_indexed_text() {
        let mut r = record("/a", "s1", 1);
        r.digest.summary = "unrelated".into();
        r.digest.text_content = "first line\nsecond has the needle here\nthird\n".into();
        let snip = content_snippet(&r.digest, "needle").unwrap();
        assert_eq!(snip.line, "second has the needle here");
        assert_eq!(&snip.line[snip.range.clone()], "needle");
    }

    #[test]
    fn content_snippet_matches_the_slug() {
        let mut r = record("/a", "s1", 1);
        r.digest.summary = "x".into();
        r.digest.slug = Some("fix-login-bug".into());
        let snip = content_snippet(&r.digest, "login").unwrap();
        assert_eq!(snip.line, "fix-login-bug");
        assert_eq!(&snip.line[snip.range.clone()], "login");
    }

    #[test]
    fn content_snippet_is_none_for_a_title_only_hit() {
        let mut r = record("/a", "s1", 1);
        r.digest.summary = "ordinary prompt".into();
        r.digest.custom_title = Some("Secret Project".into());
        r.digest.text_content = "ordinary prompt\n".into();
        // "secret" lives only in the custom title — a title-only hit, no snippet…
        assert!(content_snippet(&r.digest, "secret").is_none());
        // …yet the row is still kept, because the title matched.
        assert!(session_matches(&r, "secret", false));
    }

    #[test]
    fn content_snippet_returns_the_first_hit_only() {
        let mut r = record("/a", "s1", 1);
        r.digest.summary = "alpha needle one".into();
        r.digest.text_content = "later needle two\n".into();
        let snip = content_snippet(&r.digest, "needle").unwrap();
        assert_eq!(snip.line, "alpha needle one");
    }

    #[test]
    fn content_snippet_windows_a_long_line_around_the_hit() {
        let mut r = record("/a", "s1", 1);
        r.digest.summary = "x".into();
        r.digest.text_content = format!("{}NEEDLE{}\n", "a".repeat(200), "b".repeat(200));
        let snip = content_snippet(&r.digest, "needle").unwrap();
        // At most the window plus an ellipsis at each clipped end.
        assert!(snip.line.chars().count() <= SNIPPET_MAX_CHARS + 2);
        assert!(snip.line.starts_with(ELLIPSIS) && snip.line.ends_with(ELLIPSIS));
        assert_eq!(&snip.line[snip.range.clone()], "NEEDLE");
    }

    #[test]
    fn content_snippet_ignores_an_empty_needle() {
        let r = record("/a", "s1", 1);
        assert!(content_snippet(&r.digest, "").is_none());
    }

    proptest! {
        /// Whatever a snippet returns, its range always indexes a valid slice of
        /// its own line — never panics, never mid-character.
        #[test]
        fn content_snippet_range_is_always_a_valid_slice(
            body in "[a-zA-Z0-9 ]{0,300}",
            needle in "[a-z]{1,8}",
        ) {
            let mut r = record("/a", "s1", 1);
            r.digest.summary = String::new();
            r.digest.text_content = format!("{body}\n");
            if let Some(snip) = content_snippet(&r.digest, &needle) {
                prop_assert!(snip.line.get(snip.range.clone()).is_some());
            }
        }
    }

    #[test]
    fn project_label_is_the_last_path_component() {
        assert_eq!(project_label("/Users/me/dev/termherd"), "termherd");
    }

    #[test]
    fn project_label_ignores_a_trailing_separator() {
        assert_eq!(project_label("/Users/me/dev/termherd/"), "termherd");
    }

    #[test]
    fn project_label_handles_windows_separators() {
        assert_eq!(project_label(r"C:\Users\me\dev\termherd"), "termherd");
    }

    #[test]
    fn project_label_of_a_bare_name_is_itself() {
        assert_eq!(project_label("termherd"), "termherd");
    }

    #[test]
    fn project_label_falls_back_to_the_whole_input_when_all_separators() {
        assert_eq!(project_label("/"), "/");
        assert_eq!(project_label(""), "");
    }

    proptest! {
        /// The label is always the last separator-joined segment.
        #[test]
        fn project_label_is_the_last_joined_segment(
            segments in proptest::collection::vec("[^/\\\\]+", 1..6)
        ) {
            let path = segments.join("/");
            let last = segments.last().unwrap().as_str();
            prop_assert_eq!(project_label(&path), last);
        }

        /// A trailing separator never changes the label.
        #[test]
        fn project_label_ignores_trailing_separators(
            segments in proptest::collection::vec("[^/\\\\]+", 1..6)
        ) {
            let base = segments.join("/");
            let with_sep = format!("{base}/");
            prop_assert_eq!(project_label(&base), project_label(&with_sep));
        }

        /// Whatever comes back carries no separator (given a separator-bearing path).
        #[test]
        fn project_label_has_no_separator(
            segments in proptest::collection::vec("[^/\\\\]+", 2..6)
        ) {
            let path = segments.join("/");
            let label = project_label(&path);
            prop_assert!(!label.contains('/') && !label.contains('\\'));
        }
    }
}
