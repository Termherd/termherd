# Capture and record

Two shortcuts exist for one job: handing an AI assistant — or a bug report —
the app's exact state, without asking a human to describe it.

| | macOS | Windows / Linux | Writes |
| --- | --- | --- | --- |
| Capture state | <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> | `capture-<ts>.json` + `capture-<ts>.png` |
| Record (start / stop) | <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>R</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>R</kbd> | `capture-<ts>.gif` |

Everything lands in `~/.termherd/captures/`. `<ts>` is a UTC
`YYYYMMDD-HHMMSS-mmm` stamp, so **the latest capture is the highest-named
file** — an assistant finds it by sorting the directory, with no clock of its
own.

## The state dump

`capture-<ts>.json` is a diffable dump of the whole workspace: focus, the
resolved config, the sidebar, every tab with its panes (each pane's stable
handle, kind, cwd, status), and the focused terminal's visible text.

It is **the same model** the MCP [`snapshot`](../mcp/live-bridge.md) tool
reports, taken under a fixed full filter — one model, two readers, so a field
never means one thing on disk and another on the wire.

Because it is text, no vision is needed to read it:

```text
> Read the newest file in ~/.termherd/captures/ and tell me
> why the second pane shows no prompt.
```

## The screenshot

`capture-<ts>.png` is the real window pixels, for the render, colour and glyph
bugs a text dump cannot show. It is the companion to the JSON, not a
replacement — reach for it when the question is visual.

## The screencast

<kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>R</kbd> starts a GIF
screencast; press again to stop, or let it auto-stop at the cap. Defaults: **8
fps, 30 s, 0.5× scale**, all configurable under `record` in
[`settings.json`](../reference/settings.md).

Motion is what a still cannot carry: a gesture that half-works, a flicker, a
focus that lands on the wrong pane.

## How it stays honest

Capture is pure in the domain core — an `Event::Capture` in, an
`Effect::Capture` out — and every piece of I/O (the clock, the JSON and PNG
encoding, the files) lives in the GUI adapter. The GIF encoder runs on its own
thread, so recording does not stutter the UI or itself.

The recording state machine uses **frames as its time proxy**, not a clock,
which is what keeps it testable without one.
