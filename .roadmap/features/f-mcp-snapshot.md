+++
id = "F-mcp-snapshot"
type = "feature"
area = ["mcp", "workspace"]
status = "done"
target = ["Could"]
+++

The perception rung: a filterable, light-by-default view of the whole app.

**The perception rung.** A filterable, light-by-default `snapshot` MCP tool
exposing the whole workspace "DOM": config (font / scheme / record budget /
keymap overrides), the session-browser sidebar (projects + visible-session
counts + fold state), and the open tabs with their panes (each pane's stable
handle, kind, cwd, status), plus focus. Terminal text is **opt-in and scoped**
(`terminals` handles or `focused_terminal`, `text_lines`-truncated), so a
driving agent never pays for state it did not ask for — read the structure,
then zoom into a handle. Model + filter are pure in `core` (`core::snapshot`);
the `app` adapter injects the config bits (settings) and per-session text (the
grid) it owns, and `core` stamps the live font size. Handles are strings across
the whole surface, matching `list_sessions`. `screenshot` (#215, async window
round-trip) and single-sourcing the G1 dump (#108) onto this richer model
(#216) are follow-ups. Depends on #193; unblocks a verifiable act→observe loop
for #194
