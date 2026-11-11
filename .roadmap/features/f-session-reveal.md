+++
id = "F-session-reveal"
type = "feature"
area = ["sessions", "workspace"]
status = "done"
target = ["Could"]
+++

Reveal a session's directory and transcript in the OS file manager.

Two keymap actions, `reveal-session-dir` and `reveal-transcript` (#346),
beside the existing `Effect::OpenPath`. The transcript path comes from the
scan, never rebuilt from the project path. Torture report:
`.personal/feature-torture/reports/F-session-reveal.md`.

Shipped in #346: the scan keeps each record's transcript path, a reveal target
selects it in Finder (`open -R`) or Explorer (`explorer /select,`, passed raw so
a path with a space survives), and Linux opens the folder holding it. Both
actions are in the tab menu; each sidebar row reveals its transcript.

Not done, deliberately not ticketed: selecting the file on Linux. The
freedesktop `FileManager1.ShowItems` D-Bus call would select it, at the price
of a D-Bus dependency for one convenience; Linux opens the folder instead.
