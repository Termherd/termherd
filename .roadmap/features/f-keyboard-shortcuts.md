+++
id = "F-keyboard-shortcuts"
type = "feature"
area = ["keymap", "workspace"]
status = "done"
target = ["Must"]
+++

A configurable `KeyChord -> Action` keymap with platform-aware defaults.

Configurable keymap → actions (M3): pure `KeyChord -> Action` map in
`core::keymap` with a chord-string parser and platform-aware defaults; the
`keys` section of `settings.json` overrides any action. Drives copy/paste,
`Ctrl+Tab`/`Ctrl+Shift+Tab` cycling, close-tab, focus-search, `toggle-sidebar`
(hide pane / Ctrl+Cmd+B, #21), `scroll-top`/`scroll-bottom` (Ctrl/Cmd+Up/Down, #44),
and the in-context tab shortcuts `new-shell-here` / `new-claude-session-here`
(Ctrl/Cmd+T, Ctrl/Cmd+Alt+T, #77) + `reopen-closed-tab` (Ctrl/Cmd+Shift+T, #78,
a LIFO closed-tab stack in `core`) today; `split-*` / `focus-next/prev` bind as
those features land
