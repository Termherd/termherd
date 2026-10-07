+++
id = "F-tab-title-sync"
type = "feature"
area = ["workspace", "sessions"]
status = "todo"
target = ["Could"]
+++

A Claude tab's title follows the session name Claude holds.

The tab title follows Claude's own `/rename` and session name until the user
renames the tab by hand; clearing a manual name re-seeds the field with the
session name and resumes following (#119). Renaming a Claude tab sends
`/rename` through [F-claude-command](#f-claude-command) rather than keeping a
local copy that disagrees with Claude. Needs
[F-session-id-at-launch](#f-session-id-at-launch) for fresh tabs. Torture
report: `.personal/feature-torture/reports/F-tab-title-sync.md`.
