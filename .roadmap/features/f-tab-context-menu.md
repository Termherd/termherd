+++
id = "F-tab-context-menu"
type = "feature"
area = ["workspace", "keymap"]
status = "todo"
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
