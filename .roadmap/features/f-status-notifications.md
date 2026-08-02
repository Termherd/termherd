+++
id = "F-status-notifications"
type = "feature"
area = ["terminal", "sessions"]
status = "done"
target = ["Must"]
+++

Derive busy / waiting / attention from OSC, and surface it everywhere.

Busy / waiting / permission from OSC (M2, and #236 for the two holes that left
*every* session stuck on `starting`, making `wait_for_status` unusable): a
plain shell speaks no Claude dialect, so `pty` now injects an OSC 133
shell-integration snippet (zsh / bash / fish) and folds its prompt and command
marks into the same `SessionStatus`, with the PTY's foreground process group as
a stand-in where the injection did not take — retired by the first real signal,
and silent under ConPTY. And a Claude launch now carries a `--settings` overlay
re-enabling `CLAUDE_CODE_DISABLE_TERMINAL_TITLE`, whose `env` block in the
user's own `settings.json` outranks the environment we spawn with and silenced
the only channel we had: the `pty` reader decodes the raw byte stream with
`termherd_claude::osc`; busy/idle titles plus an OSC 9 notification → a
distinct `Attention` status (sticky over idle, cleared by work). Surfaced as a
badge on the focused terminal, a per-session dot in the sidebar, and a dot on
each tab (with `F-session-tabs`); the bell is decoded but not treated as
activity. #86: `core` now tracks OS window focus (`Event::WindowFocusChanged`)
so a background tab's notification still reaches the OS while termherd itself
is focused — only the active tab's focused pane skips the banner. #244 closed
the one place the injection turned on itself: the private `ZDOTDIR` it hands
zsh is inherited by everything descending from that shell, so a termherd
launched from a termherd shell read it as the user's home and generated a
`.zshenv` that sourced itself — the shell never reached a prompt, and the
session sat on `starting`, the very symptom the paragraph above exists to
remove. A generated directory is now recognised by name and never replayed
from, and the recipe exports the home it displaced so a nesting level cannot
lose it
