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
(hide pane / Cmd+B, #21), `scroll-top`/`scroll-bottom` (Cmd+Up/Down, #44),
and the in-context tab shortcuts `new-shell-here` / `new-claude-session-here`
(Cmd+T, Cmd/Ctrl+Alt+T, #77) + `reopen-closed-tab` (Cmd+Shift+T, #78, a LIFO
closed-tab stack in `core`) today; `split-*` / `focus-next/prev` bind as those
features land

**Off macOS, no default takes a bare Ctrl+letter (#404, #406).** Those are the
terminal's control keys, and a bound chord never reaches the pane, so the
Windows/Linux defaults moved to Ctrl+Shift+letter (Windows Terminal's
convention), scroll to Ctrl+Shift+Home/End and the splits to Alt+Shift+Plus /
Minus / D. Bare Ctrl+V pastes on Windows only. `Platform` tells Windows from
Linux, and a test in `pty` runs every default through the encoder.

Still open after #406, as of 2026-11-01: Ctrl+1…9 and Ctrl+0/-/= still sit on
bare Ctrl off macOS (#323); `press_keys` names Home/End now but not PageUp,
Delete, Backspace, Space, Insert or the function keys (#324). Deliberately not
ticketed: Alt+Shift+- reaches AZERTY only from the keypad (Alt+Shift+D covers
it), Ctrl+Shift+numpad 7 / 1 with NumLock on scrolls instead of typing, and
the pane-side guard test checks bare Ctrl only, not Ctrl+Alt (AltGr on
Windows). A Windows run by hand (Ctrl+D, Alt+V into Claude, the Alt+Shift
splits) has not happened yet.
