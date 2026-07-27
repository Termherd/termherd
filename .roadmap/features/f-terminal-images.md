+++
id = "F-terminal-images"
type = "feature"
area = ["terminal"]
status = "todo"
target = ["Should"]
+++

Render images inline in the terminal — parked, no demand and no cheap slice.

Render images inline in the terminal (iTerm2 OSC 1337 / Sixel / Kitty
graphics), sibling to `F-jsonl-viewer` / `F-file-diff-panel` in the rendering
family. Filed as #85. **Parked** (feature-torture ⏸ `F-terminal-images.md`):
the issue's stated symptom ("garbage escape text") doesn't reproduce —
`vte`/`alacritty_terminal` already discards unrecognised OSC/DCS/APC sequences
cleanly; the real gap is silence, not garbage. No slice is cheap: even a
placeholder-only render needs the same chunked-payload reassembly
`crates/claude/src/osc.rs` explicitly punts on today, across 3 mutually
incompatible protocols (OSC/ DCS/APC). Zero demand signal beyond the filed
issue. Revisit on a real user report of the silent drop, or a free cycle after
`F-terminal-split` (#54/#55)
