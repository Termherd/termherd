+++
id = "F-close-on-exit"
type = "feature"
area = ["terminal", "workspace"]
status = "done"
target = ["Should"]
+++

A pane whose shell exits cleanly closes itself; a failed one stays readable.

Auto-close a pane/tab on clean exit (#185, shipped in #187): a PTY exiting with
code 0 (the user typed `exit` at a prompt) closes its pane — collapse the
split, or close the tab (onto the reopen stack) when it was the last pane; an
emptied workspace stays open, termherd never quits. Non-zero/unknown exits keep
the dead-terminal view so errors stay readable. Quitting Claude still never
closes the tab — structurally: `claude` is typed *into* a shell, so its exit
returns to the prompt with the PTY alive (the planned `Launch::Claude` gate
proved redundant and was dropped mid-review). Ship also fixed exit detection on
Windows: ConPTY never delivers reader EOF on a child's natural exit, so the
`pty` adapter reaps in a dedicated waiter thread. Fixed policy, no settings
knob
