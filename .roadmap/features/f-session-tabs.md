+++
id = "F-session-tabs"
type = "feature"
area = ["terminal", "workspace"]
status = "done"
target = ["Must"]
+++

Every launched session is a tab, with its own badge, title and drag order.

Tabbed open sessions (M3): every launched session is a tab; a tab strip
switches between them, each chip carrying its activity dot (the FR8 tab badge)
and a close button that kills the session's PTY — the first UI-driven
`Effect::Kill`. Tab tree edits (`activate`/`close_tab`, most-urgent
`tab_status`) are pure in `core`. Tab labels: a resumed tab takes the session
name from the scanned digest (#109/#118) — current Claude (2.1.220) *does* emit
an OSC-0 title, but reports its own product name (`✳ Claude Code`) until it has
something session-specific to say, and #236 filters that as no title at all, so
the OSC-0 override (#24) still does not fire there; a fresh/unscanned session
keeps the `<repo>` kind label. The OSC plumbing stays in place and still wins
where a Claude does emit a real title: the `osc` decoder carries the title
text, the `pty` reader forwards a change as `PtyEvent::Title`, and
`Workspace::set_session_title` relabels the hosting tab — which also lets a
sidebar rename retitle the open tab live. Reflecting Claude's *own* `/rename`
and live task name is tracked as #119. Hovering a tab shows the session's
fuller description — the same hover card the sidebar uses for a resumed
session, a title + cwd card otherwise (#76, `App::tab_record` resolves the
record so the two surfaces stay single-sourced). Drag-reorder (FR5) and
keyboard switching (`Ctrl+Tab`, via `F-keyboard-shortcuts`) both ship.
Double-clicking a chip renames the tab inline (#145): a durable
`Tab.custom_title` overlays the derived title (`Tab::display_title` resolves
the precedence, so a later OSC/digest relabel never masks a custom name), a
blank name reverts to the derived title, blur commits and Escape cancels. The
tab-strip view was extracted to its own `shell/view/tabs.rs` for this (C2 of
the intra-crate refactor, #168)
