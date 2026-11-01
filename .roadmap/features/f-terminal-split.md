+++
id = "F-terminal-split"
type = "feature"
area = ["workspace", "keymap"]
status = "done"
target = ["Should"]
+++

Split panes with directional focus and drag-adjustable ratios.

Split panes (h/v), focus, resize (moved from Must, PRD rev. 5): the **#54 MVP
shipped** — the `split-*`/`focus-*`/`close-focused` keymap actions now drive
`core` (recursive iced pane rendering from the `Workspace` tree, fixed-ratio
50/50 splits, per-leaf PTY geometry), plus click-to-focus (`Event::FocusPane`)
and directional keyboard focus that cycles within its axis (`Event::FocusDir`,
`mod+shift+arrows`); `mod+w` collapses the focused pane rather than the whole
tab. Default binds: `mod+d` / `mod+shift+d` split, `mod+shift+arrows` focus.
**Drag-resize (#55) shipped:** draggable handles adjust each split from 10% to
90% of its usable extent. `core::Workspace` remains the single source of
truth, and the affected tab's PTYs resize on release.
