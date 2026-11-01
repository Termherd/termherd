# Issue #108 — capture state for the AI dev loop

> Rung 0+1 of the `F-capture` ladder (ROADMAP). One keybind snapshots the
> running app into a diffable JSON state dump **and** a real PNG, so an AI
> assistant can read the current state and tighten the fix loop.

## What it does

Press **⌘⇧S** (macOS) / **Ctrl+Shift+S** elsewhere — rebindable in
`settings.json` as `capture` — to write two artefacts to
`~/.termherd/captures/`:

- `capture-<ts>.json` — a deterministic, *diffable* state dump. No vision
  needed: focus, resolved config, the sidebar, every tab with its panes, and the
  focused terminal's visible text.
- `capture-<ts>.png` — the real window pixels (iced `window::screenshot`), for
  render / colour / glyph bugs the text dump can't show.

`<ts>` is a UTC `YYYYMMDD-HHMMSS-mmm` stamp, so the **newest capture is the
highest filename** — an AI finds the latest by sorting the directory.

## Flow — pure `core`, all I/O in `app`

```text
        ⌘⇧S  ──>  Shell::capture()
                       │ gathers the adapter-owned inputs: resolved config
                       │ + the focused pane's visible grid as text
                       ▼
   ┌───────────────────────────────────────────────┐
   │  core (pure: no I/O, no clock, no panic)        │
   │                                                 │
   │  Event::Capture(SnapshotInputs)                 │
   │            │                                    │
   │            ▼   App::build_capture()             │
   │              = App::snapshot(capture filter)    │
   │  Effect::Capture(WorkspaceSnapshot) ────────────┼──┐
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

The dump **is** the `WorkspaceSnapshot` the MCP `snapshot` tool reports — one
model, two readers, so a field can never mean one thing on disk and another on
the wire. A capture just fixes the filter (`SnapshotFilter::capture()`): every
structural section, plus the focused pane's screen kept whole (a file pays for
the full picture where a streamed call must not).

`App::build_capture` is therefore a call to the snapshot builder. The parts the
pure core cannot read — the resolved config (settings live in `app`) and the
terminal text (the grid lives in `pty`) — ride in on the event as
`SnapshotInputs`:

```text
WorkspaceSnapshot
├─ focus                                // always present
│    ├─ tab     : Option<usize>         // None when no tab is open
│    └─ session : Option<u64>           // focused pane's stable handle
├─ config  : Option<ConfigSummary>      // font size, scheme, record budget, …
├─ sidebar : Option<SidebarSnapshot>    // filter knobs + one row per project
├─ tabs    : Option<Vec<TabSnapshot>>
│    ├─ active : bool
│    ├─ title  : String
│    ├─ status : Option<SessionStatus>  // most-urgent of the tab's sessions
│    └─ panes  : Vec<PaneSnapshot>      // left→right: handle, kind, cwd, status
└─ terminals : BTreeMap<u64, String>    // scoped text by handle (here: focused)
```

## Example dumps

Two tabs; the active one is a split with pane `3` focused:

```json
{
  "focus": { "tab": 1, "session": "3" },
  "config": { "font_size": 14.0, "terminal_scheme": "gruvbox-dark",
              "record_fps": 8, "record_scale": 0.5, "keymap_overrides": 0 },
  "sidebar": { "hidden": false, "search": "", "search_titles_only": false,
               "show_archived": false,
               "projects": [ { "path": "/Users/me/dev/termherd",
                               "session_count": 4, "collapsed": false } ] },
  "tabs": [
    { "active": false, "title": "termherd $", "status": "idle",
      "panes": [ { "handle": "1", "kind": "shell",
                   "cwd": "/Users/me/dev/termherd", "status": "idle" } ] },
    { "active": true, "title": "termherd 🤖", "status": "busy",
      "panes": [ { "handle": "2", "kind": "claude",
                   "cwd": "/Users/me/dev/termherd", "status": "idle" },
                 { "handle": "3", "kind": "shell",
                   "cwd": "/Users/me/dev/termherd", "status": "busy" } ] }
  ],
  "terminals": {
    "3": "$ cargo test\n   Compiling termherd-core\ntest result: ok. 146 passed"
  }
}
```

Empty workspace (nothing launched yet) — absent sections and an empty
`terminals` map are omitted entirely:

```json
{ "focus": { "tab": null, "session": null }, "config": { … },
  "sidebar": { … }, "tabs": [] }
```

Field rules:

- **Handles are strings** (`"3"`), everywhere, matching the MCP `list_sessions`
  and `snapshot` surface. The tab index in `focus` stays a number.
- `status` is the agent-facing vocabulary shared with MCP: `starting` / `busy` /
  `idle` / `attention` / `exited`. (It used to be the UI badge wording, where
  `Idle` read `ready`; single-sourcing the model retired that second vocabulary.)
- `panes` are the tab's leaves left to right — one for a plain tab, several for
  a split — each with its stable handle, `shell`/`claude` kind, cwd and status.
- `terminals` is keyed by handle and holds only the **focused** pane's visible
  text (`\n`-joined rows, trailing blanks trimmed), kept whole. It is absent
  when nothing is focused.

## Design decisions

- **JSON encoding lives in `app`, not `core`.** `core` carries no serde
  dependency and the issue forbids new deps, so the `Effect` carries the
  structured snapshot and the adapter owns the wire form — one shared mirror
  (`crates/app/src/snapshot_dto.rs`) for both the dump and the MCP tool. Pathing
  (the timestamp, the home dir) is likewise an `app` concern — so `app`, not `core`,
  names the files. The issue's "Effect carrying the target path" became "`app`
  owns paths" because a path needs the clock + home dir that `core` must not
  touch.
- **Text captures *what*; PNG captures *how it looks*.** The dump records pane
  *membership* + focus + status + PTY text; split direction/ratio are
  deliberately left to the pixel rung. Different bug classes, one shared
  keybind.

## Tests

- **core:** the dump carries every section; the focused text rides whole; a
  non-focused pane's injected text stays out; a tab's custom title wins over its
  derived one; empty workspace; split pane order; the capture filter itself;
  keymap `mod+shift+s` ↔ `capture`.
- **app:** `stamp` formats a known UTC instant and sorts chronologically; the
  JSON is the MCP shape (string handles, `idle`); `write_dump`; `write_png`
  round-trips dimensions; shell-level capture writes the whole-workspace JSON
  (driven through a dir seam — env mutation is `unsafe` in edition 2024, which
  the crate denies).

Not exercised headless: the actual PNG, which needs a live iced window — verify
by running the app.

## Files

- `crates/core/src/snapshot.rs` — the shared model and `SnapshotFilter::capture`.
- `crates/core/src/app.rs` — `Event::Capture`, `Effect::Capture`.
- `crates/core/src/app/capture.rs` — `App::build_capture`.
- `crates/core/src/keymap.rs` — `Action::Capture`, default `mod+shift+s`.
- `crates/app/src/capture.rs` — stamp, JSON encode, output dir. The PNG encoder
  itself moved to `crates/app/src/image.rs` when the MCP `screenshot` tool
  became its second reader; `write_png` is now the disk wrapper over it.
- `crates/app/src/snapshot_dto.rs` — the JSON wire form, shared with the MCP
  handler.
- `crates/app/src/paths.rs` — shared `home_dir` / `termherd_dir` the stores
  resolve through (one `~/.termherd` resolver, not seven).
- `crates/app/src/shell.rs` — `Shell::capture` / `perform_capture`, the
  `CaptureScreenshot` → off-thread `CaptureWritten` PNG encode.
