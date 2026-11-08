+++
id = "F-session-reveal"
type = "feature"
area = ["sessions", "workspace"]
status = "todo"
target = ["Could"]
+++

Reveal a session's directory and transcript in the OS file manager.

Two keymap actions, `reveal-session-dir` and `reveal-transcript` (#346),
beside the existing `Effect::OpenPath`. The transcript path comes from the
scan, never rebuilt from the project path. Torture report:
`.personal/feature-torture/reports/F-session-reveal.md`.
