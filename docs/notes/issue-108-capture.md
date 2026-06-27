# Issue #108 — capture state for the AI dev loop

> Rung 0+1 of the `F-capture` ladder (ROADMAP). One keybind snapshots the
> running app into a diffable JSON state dump **and** a real PNG, so an AI
> assistant can read the current state and tighten the fix loop.

## What it does

Press **⌘⇧S** (macOS) / **Ctrl+Shift+S** elsewhere — rebindable in
`settings.json` as `capture` — to write two artefacts to
`~/.termherd/captures/`:

- `capture-<ts>.json` — a deterministic, *diffable* state dump. No vision
  needed: tabs, focus, per-tab activity, pane membership, and the focused
  terminal's visible text.
- `capture-<ts>.png` — the real window pixels (iced `window::screenshot`), for
  render / colour / glyph bugs the text dump can't show.

`<ts>` is a UTC `YYYYMMDD-HHMMSS-mmm` stamp, so the **newest capture is the
highest filename** — an AI finds the latest by sorting the directory.

## Flow — pure `core`, all I/O in `app`

```text
        ⌘⇧S  ──>  Shell::capture()
                       │ reads focused pane's visible grid as text
                       ▼
   ┌───────────────────────────────────────────────┐
   │  core (pure: no I/O, no clock, no panic)        │
   │                                                 │
   │  Event::Capture { focused_pty_text }            │
   │            │                                    │
   │            ▼   App::build_capture()             │
   │  Effect::Capture(CaptureDump) ──────────────────┼──┐
   └─────────────────────────────────────────────────┘  │
                                                          ▼
                              app adapter (crates/app/src/capture.rs)
                                    │ stamp(UTC) + serde JSON + png encode
            ┌───────────────────────┴───────────────────────┐
            ▼                                                ▼
  ~/.termherd/captures/                          window::latest()
    capture-<ts>.json   (written now, sync)         .and_then(screenshot)
                                                         │
                                                         ▼
                                          capture-<ts>.png  (written when
                                                             iced returns pixels)
```

The capture event is the only one whose effect the shell performs specially
(not through the generic effect loop): the JSON and PNG must share one
timestamp, and the PNG needs an async `window::screenshot` follow-up the
fire-and-forget loop can't return.

## The `core` model

`App::build_capture` folds the workspace, the live-session statuses, and the
shell-injected PTY text into one pure value (the grid itself lives in the `pty`
adapter, so the shell hands it in via the event):

```text
CaptureDump
├─ active_tab : Option<usize>          // None when no tab is open
├─ tabs : Vec<CaptureTab>
│    ├─ active        : bool
│    ├─ title         : String
│    ├─ status        : Option<SessionStatus>   // most-urgent of the tab's sessions
│    ├─ sessions      : Vec<u64>                // pane leaves, left→right
│    └─ focus_session : Option<u64>             // set ONLY on the active tab
└─ focused_pty : Option<String>        // focused terminal's visible text
```

## Example dumps

Two tabs; the active one is a split with pane `3` focused (`focus_session` is
emitted only for the active tab):

```json
{
  "active_tab": 1,
  "tabs": [
    { "active": false, "title": "termherd $", "status": "idle", "sessions": [1] },
    {
      "active": true,
      "title": "termherd 🤖",
      "status": "busy",
      "sessions": [2, 3],
      "focus_session": 3
    }
  ],
  "focused_pty": "$ cargo test\n   Compiling termherd-core\ntest result: ok. 146 passed"
}
```

Empty workspace (nothing launched yet):

```json
{ "active_tab": null, "tabs": [], "focused_pty": null }
```

Field rules:

- `status` is one of `starting` / `busy` / `idle` / `attention` / `exited` (the
  most urgent among a tab's sessions).
- `sessions` are the tab's panes left to right — one id for a plain tab, several
  for a split.
- `focused_pty` is the focused terminal's visible text (`\n`-joined rows,
  trailing blanks trimmed), or `null` when nothing is focused.

## Design decisions

- **JSON encoding lives in `app`, not `core`.** `core` carries no serde
  dependency and the issue forbids new deps, so the `Effect` carries the
  structured `CaptureDump` and the adapter owns the wire form. Pathing (the
  timestamp, the home dir) is likewise an `app` concern — so `app`, not `core`,
  names the files. The issue's "Effect carrying the target path" became "`app`
  owns paths" because a path needs the clock + home dir that `core` must not
  touch.
- **Text captures *what*; PNG captures *how it looks*.** The dump records pane
  *membership* + focus + status + PTY text; split direction/ratio are
  deliberately left to the pixel rung. Different bug classes, one shared
  keybind.

## Tests (10)

- **core (4):** dump snapshots tabs/focus/status/pty-text; empty-workspace
  dump; split pane membership in order; `SessionStatus::as_str`; keymap
  `mod+shift+s` ↔ `capture`.
- **app (6):** `stamp` formats a known UTC instant and sorts chronologically;
  JSON shape (incl. `focus_session` omitted on the inactive tab); `write_dump`;
  `write_png` round-trips dimensions; shell-level capture writes the JSON with
  the focused PTY text (driven through a dir seam — env mutation is `unsafe` in
  edition 2024, which the crate denies).

Not exercised headless: the actual PNG, which needs a live iced window — verify
by running the app.

## Files

- `crates/core/src/capture.rs` — `CaptureDump` / `CaptureTab`,
  `SessionStatus::as_str`.
- `crates/core/src/app.rs` — `Event::Capture`, `Effect::Capture`,
  `App::build_capture`.
- `crates/core/src/keymap.rs` — `Action::Capture`, default `mod+shift+s`.
- `crates/app/src/capture.rs` — stamp, JSON encode, PNG encode, output dir.
- `crates/app/src/shell.rs` — `Shell::capture` / `perform_capture`, the
  `CaptureScreenshot` message.
