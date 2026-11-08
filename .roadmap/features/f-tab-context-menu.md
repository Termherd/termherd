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

Shipped (#340): the menu, bound to `mod+shift+m`. A first `mod+.` binding
never fired on French AZERTY: a chord is named from the key without Shift,
so `.` (Shift+`;` there) arrives as `;`; a letter is reachable on every
layout. A
right-click focuses the tab first, since every entry acts on focus, and the
menu is anchored on that pane: it closes when the pane loses focus. It is a
`tab-menu` rung on the keyboard ladder that answers the arrows, `enter` and
`escape` itself, so MCP can drive it end to end; `enter` reports the verdict
of the entry it ran. Entries today: rename tab, tab colour… (only where a pick
can apply, added by #343), copy agent name (only when the action would find a
name), new shell / new Claude session here, split right / down, close pane.
The list is data in `shell::tab_menu`: each later entry (reveal, history, send
to Claude Desktop) is one line there once its action exists. Not yet: the menu opens
centred rather than beside the tab, and a screen reader cannot see it.
