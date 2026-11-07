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
handle, kind, cwd, status, and the `pid`, `peer_name` and `session_id` of
the Claude running in it, all three `null` when none is, and the `color`
it wears — its Claude's `/color`, or the colour picked for a shell's tab),
and the focused terminal's visible text, its secrets masked
([below](#what-a-capture-keeps-private)).

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

## What a capture keeps private

A capture holds a whole terminal's text, and a terminal shows secrets: a
pasted token, an `env` listing, a `.env` file someone printed.

- **Only you can read it.** The directory is `0700` and every file in it
  `0600`, on macOS and Linux; a directory an older build left open is
  tightened at the next capture. On Windows the files inherit your profile's
  permissions.
- **Secrets are masked in the dump.** Before the JSON is written, the
  terminal text has each of these replaced by `[redacted]`:

  | Shape | Example | Kept |
  | --- | --- | --- |
  | AWS access key id | `AKIA…`, `ASIA…` | — |
  | AWS secret assignment | `AWS_SECRET_ACCESS_KEY=…`, any `AWS_` name holding `KEY`, `SECRET` or `TOKEN` | the name |
  | API key | `sk-…` (OpenAI, Anthropic) | — |
  | GitHub token | `ghp_…`, `gho_…`, `ghu_…`, `ghs_…`, `ghr_…`, `github_pat_…` | — |
  | HTTP bearer token | `Authorization: Bearer …` | `Bearer` |
  | PEM private key | `-----BEGIN … PRIVATE KEY-----` block | the `BEGIN` and `END` lines |

  Turn it off with `"capture": { "redact": false }` in
  [`settings.json`](../reference/settings.md); an MCP client cannot change
  it. The screenshot and the screencast are pictures and are **not**
  masked: a secret on screen is in them.

  Only these shapes are recognised. A password or a token in some other
  format is written as-is: there is no guess based on how random a string
  looks, because hashes, UUIDs and base64 output would be masked with it.
- **Only the newest 50 captures are kept.** After each capture or
  screencast, the oldest ones beyond 50 are deleted; a JSON dump and its
  screenshot count as one. Only files named like TermHerd's own
  (`capture-YYYYMMDD-HHMMSS-mmm.json`, `.png` or `.gif`) are ever deleted:
  anything else you put in the directory, or a capture you rename, stays.
  Copy a capture out of the directory to keep it.

## How it stays honest

Capture is pure in the domain core — an `Event::Capture` in, an
`Effect::Capture` out — and every piece of I/O (the clock, the JSON and PNG
encoding, the files) lives in the GUI adapter. The GIF encoder runs on its own
thread, so recording does not stutter the UI or itself.

The recording state machine uses **frames as its time proxy**, not a clock,
which is what keeps it testable without one.
