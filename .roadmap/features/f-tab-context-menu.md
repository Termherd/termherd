+++
id = "F-tab-context-menu"
type = "feature"
area = ["workspace", "keymap"]
status = "done"
target = ["Could"]
+++

A per-tab action menu, from a right-click or an `open-tab-menu` action.

An in-app overlay rather than a native OS menu, so `press_keys` / `run_action`
can drive it (#340). Every entry is an existing keymap action; the entries
arrive with their own features — [F-keymap-rename-tab](#f-keymap-rename-tab),
[F-session-accent-colors](#f-session-accent-colors),
[F-session-reveal](#f-session-reveal), [F-prompt-history](#f-prompt-history),
[F-copy-agent-name](#f-copy-agent-name),
[F-session-send-desktop](#f-session-send-desktop). Torture report:
`.personal/feature-torture/reports/F-tab-context-menu.md`.

Shipped (#340): the menu, bound to `mod+.` (Ctrl+. has no control code in a
legacy terminal encoding, so it takes no key from a program in a pane). A
right-click focuses the tab first, since every entry acts on focus. It is a
`tab-menu` rung on the keyboard ladder that answers the arrows, `enter` and
`escape` itself, so MCP can drive it end to end. Entries today: rename tab,
copy agent name (only where the focused pane runs Claude), new shell / new
Claude session here, split right / down, close pane. The list is data in
`shell::tab_menu`: each later entry (colour, reveal, history, send to Claude
Desktop) is one line there once its action exists. Not yet: the menu opens
centred rather than beside the tab, and a screen reader cannot see it.
