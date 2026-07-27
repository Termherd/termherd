+++
id = "F-terminal-split"
type = "feature"
area = ["workspace", "keymap"]
status = "todo"
target = ["Should"]
+++

Split panes with directional focus; drag-resize is what remains.

Split panes (h/v), focus, resize (moved from Must, PRD rev. 5): the **#54 MVP
shipped** — the `split-*`/`focus-*`/`close-focused` keymap actions now drive
`core` (recursive iced pane rendering from the `Workspace` tree, fixed-ratio
50/50 splits, per-leaf PTY geometry), plus click-to-focus (`Event::FocusPane`)
and directional keyboard focus that cycles within its axis (`Event::FocusDir`,
`mod+shift+arrows`); `mod+w` collapses the focused pane rather than the whole
tab. Default binds: `mod+d` / `mod+shift+d` split, `mod+shift+arrows` focus.
What remains is **drag-resize (#55, blocked-by #54)** to flip the fixed ratio;
`core::Workspace` stays the single source of truth throughout
