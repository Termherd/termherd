+++
id = "F-session-accent-colors"
type = "feature"
area = ["workspace", "sidebar"]
status = "done"
target = ["Could"]
+++

A per-session accent on its tab and sidebar row.

Per-session visual accents: a colour on a session's tab chip and sidebar row,
so parallel sessions are distinguishable at a glance. The pane border inside a
split tab is [F-pane-accent-border](#f-pane-accent-border). Chrome
accents, not grid colours — sibling of, but separate from,
`F-terminal-palette`. The kind is shown by
[F-tab-kind-icon](#f-tab-kind-icon), so colour stays free for the session.

Scoped into two slices. For a Claude tab the colour is the one Claude Code's
`/color` set — the last `agent-color` entry in the transcript — with no local
copy (#342). Picking a colour sends `/color` to a Claude tab through
[F-claude-command](#f-claude-command) and stores it on a shell tab, which
Claude knows nothing of (#343). Both use the same eight-colour palette as
`/color`.

Slice 1 shipped (#342): the digest keeps the last `agent-color` value, an
open tab follows it at the next rescan, and the focused pane decides a split
tab. A Claude tab is outlined and its sidebar row barred in the colour, and
the hover card names it — the cue for anyone who cannot tell red from green.
The MCP `snapshot` reports it per pane.

Slice 2 shipped (#343): the tab menu's *Tab colour…* and a `pick-tab-color`
action open the palette, plus *None*, over the focused tab. Who keeps a
pane's colour is decided by its launch, the rule the slash-command check
uses: a pick for a Claude pane is typed as `/color` behind the confirmation,
and a shell tab stores it in `core` until the tab closes. A Claude launch
whose Claude has exited counts as a shell, so `/color` is never typed into
one. Torture report:
`.personal/feature-torture/reports/F-session-accent-colors.md`.
