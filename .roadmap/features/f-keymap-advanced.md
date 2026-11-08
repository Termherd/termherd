+++
id = "F-keymap-advanced"
type = "feature"
area = ["keymap", "terminal"]
status = "todo"
target = ["Backlog"]
+++

The keymap concerns from the gist that still need design.

Keymap concerns from the gist that need design, layered on the shipped
`F-keyboard-shortcuts`:

- ~~localized number-row handling (AZERTY: `&`→1, `é`→2, …) so Ctrl/Cmd+Number
  (issue #26) works on non-QWERTY layouts~~ — **done** with #26: the number
  row is matched by physical key position, so Ctrl/Cmd+1…9 land on the same
  keys on every layout (QWERTY/AZERTY/QWERTZ/…)
- per-command keymap configuration (different bindings per running command)
  — **stays design-first** (feature-torture 🧬 `F-keymap-advanced.md`):
  was blocked on foreground-process detection. macOS and Linux read the PTY
  foreground process group (#236); #143 added the alternate screen as the
  Windows ConPTY fallback for the close confirmation, so a non-full-screen
  command is still invisible there; file only once that's designed
- ~~a configurable "bypass" key so a modifier passes through to the terminal
  instead of the app (cf. Ghostty `macos-option-as-alt`)~~ — **done** with
  #59: `terminal.option_as_meta` (`none` / `left` / `right` / `both`, default
  `left`) picks which macOS Option key is Meta; the other one types the glyph
  the layout composes
