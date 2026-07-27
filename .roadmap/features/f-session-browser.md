+++
id = "F-session-browser"
type = "feature"
area = ["sidebar", "sessions"]
status = "done"
target = ["Must"]
+++

Scan, group and list `~/.claude` sessions, live as the filesystem changes.

Scan + derive + group + list + live fs-watch updates (debounced `notify`, FR2);
a per-project disclosure triangle folds its session list, persisted to
`~/.termherd/collapsed.json` (#22); long groups list only the N most recent
sessions with a "… N more" expander (`sidebar.session_limit` in
`settings.json`, default 5, 0 = all; #131). Section headers fold on a title
click, not only the disclosure triangle — the Favorites and Plans & mémoire
titles gained the parity a project header already had, via a shared
`section_header` builder (#146). Thin theme-aware rules separate the sidebar
sections (Favorites / Plans & mémoire / Projects) so the grouping reads at a
glance (#150). The sidebar view was extracted to its own
`shell/view/sidebar.rs` with per-section row builders, dropping the
`too_many_lines` allow (C2 of the intra-crate refactor, #168)
