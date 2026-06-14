# TermHerd

> A Rust replatform experiment for a Claude Code session
> workspace. Native, terminal-multiplexer-style (tabs + splits + keyboard
> driven), with the quality bar the predecessor lacked.

Inspired by [doctly/switchboard](https://github.com/doctly/switchboard), the
Electron app it replatforms; see [`docs/background/`](docs/background/) for
the full reasoning archive.

This is an early scaffold. Status, scope, and design live in:

- [`docs/PRD.md`](docs/PRD.md) — Product Requirements Document
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — Architecture
- [`ROADMAP.md`](ROADMAP.md) — feature buckets (MoSCoW)
- [`CHANGELOG.md`](CHANGELOG.md)

## Run

```bash
cargo run -p termherd-app
```

## Configuration

Optional user settings live in `~/.termherd/settings.json` (on Windows,
`%USERPROFILE%\.termherd\settings.json`). The file is read at startup; if it
is missing or invalid, TermHerd falls back to defaults rather than refusing to
start. There is no in-app settings panel yet — edit the file and restart.

```json
{
  "shell": { "program": "pwsh", "args": [] },
  "theme": "dark"
}
```

- `shell` — the shell launched for each session. Omit it (or set it to `null`)
  to use the platform default login shell; `args` is optional.
- `theme` — `"dark"` (default) or `"light"`, for the GUI chrome (sidebar, tab
  strip, buttons). The terminal grid keeps its own colours.

Window size and position persist separately to `~/.termherd/window.json`.

## Test

```bash
cargo test --workspace
```

## CI gates (mirror locally before pushing)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check         # if cargo-deny installed
```

## Toolchain

Pinned to **rust 1.95.0** via `rust-toolchain.toml`. Edition 2024.

## Layout

```text
crates/core    — domain, headless App, workspace (pane tree), keymap, ports
crates/claude  — Claude CLI format codec (path encode/derive, JSONL)  [pure]
crates/app     — iced GUI shell (M3+); currently a tracing+single-instance stub
```

The hexagonal dependency rule: `app` → `core` ← `adapters` (and `core` →
`claude`). `core` depends on nothing concrete.
