+++
id = "F-tab-title-sync"
type = "feature"
area = ["workspace", "sessions"]
status = "done"
target = ["Could"]
+++

A Claude tab's title follows the session name Claude holds.

Shipped in #119. One resolver in `core` ranks a tab's title sources —
the session's name (Claude's `/rename` or a name kept in the sidebar,
whichever was given last) >
the live OSC title > Claude's AI title or first prompt > the launch label —
and every open tab re-resolves on a rescan, a metadata load, a sidebar
rename and a re-key. That reaches fresh tabs too, through the id minted by
[F-session-id-at-launch](#f-session-id-at-launch).

Renaming a Claude tab (double-click, the MCP `rename_tab`, or the sidebar ✎
on an open session) arms `/rename` through
[F-claude-command](#f-claude-command) rather than keeping a local copy that
disagrees with Claude; a refusal shows a notice under the tab strip. A shell
tab keeps its local name (#145), and so does a Claude tab whose Claude has
exited. Once cleared, the rename field shows the name a blank commit leaves.
The "follow by default" setting was dropped as YAGNI. Torture report: `.personal/feature-torture/reports/F-tab-title-sync.md`.
