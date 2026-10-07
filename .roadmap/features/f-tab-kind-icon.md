+++
id = "F-tab-kind-icon"
type = "feature"
area = ["workspace"]
status = "done"
target = ["Could"]
+++

A kind mark beside each tab's status dot, instead of a glyph in its title.

Each tab chip shows `✳` (Claude) or `❯` (shell) between its status dot and its
title, read from the focused pane's `Launch` through `App::tab_kind` (#341).
The title no longer carries `$` / `🤖`: a fresh tab is titled after its
project, so a rename opens on a clean name, a Claude retitle cannot drop the
mark, and the MCP `title` carries no presentation noise (`kind` already said
it). Lands ahead of [F-tab-title-sync](#f-tab-title-sync), which rewrites the
same title policy. Torture report:
`.personal/feature-torture/reports/F-tab-kind-icon.md`.
