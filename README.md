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

## Install

Each tagged release publishes desktop installers on the
[Releases](https://github.com/bastien-gallay/termherd/releases) page. Pick the
one for your platform:

- **macOS** — download `TermHerd_<version>_<arch>.dmg`, open it, and drag
  **TermHerd** into Applications. The build is not yet notarized (signing is
  pending, see the roadmap), so on first launch right-click the app and choose
  **Open**, or clear the quarantine flag:
  `xattr -dr com.apple.quarantine /Applications/TermHerd.app`.
- **Windows** — run the `*-setup.exe` (NSIS installer). Because it is unsigned
  for now, SmartScreen may warn — choose **More info → Run anyway**.
- **Linux** — install the `.deb`
  (`sudo apt install ./termherd_<version>_amd64.deb`), or download the
  `.AppImage`, `chmod +x` it, and run it directly.

Prefer a bare command-line binary? The same releases carry one-line installers
that drop `termherd` into your Cargo bin directory:

```bash
# macOS / Linux
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/bastien-gallay/termherd/releases/latest/download/termherd-installer.sh | sh
```

```powershell
# Windows
powershell -c "irm https://github.com/bastien-gallay/termherd/releases/latest/download/termherd-installer.ps1 | iex"
```

## Run from source

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
  "theme": "dark",
  "keys": {
    "copy": "ctrl+c",
    "paste": ["ctrl+v", "ctrl+shift+v"],
    "next-tab": "ctrl+tab",
    "activate-tab-1": "ctrl+1"
  }
}
```

- `shell` — the shell launched for each session. Omit it (or set it to `null`)
  to use the platform default login shell; `args` is optional.
- `theme` — `"dark"` (default) or `"light"`, for the GUI chrome (sidebar, tab
  strip, buttons). The terminal grid keeps its own colours.
- `keys` — keyboard overrides. Each entry binds an action to one chord or a
  list of chords (`"ctrl+shift+c"`, order/case-insensitive; modifiers `ctrl`,
  `shift`, `alt`, `cmd`). An entry replaces that action's default; unspecified
  actions keep their per-platform defaults. Unknown actions and bad chords are
  ignored. Actions: `copy`, `paste`, `next-tab`, `prev-tab`, `close-focused`,
  `focus-search`, `toggle-sidebar` (hide / restore the session browser,
  Ctrl/Cmd+B), and `activate-tab-1` … `activate-tab-9` (jump straight to the
  Nth open tab).

Window size and position persist separately to `~/.termherd/window.json`, and
session stars / archives / custom titles to `~/.termherd/metadata.json` (an
overlay — TermHerd never writes under `~/.claude`). Star (★), archive (⊟) and
rename (✎) are buttons on each sidebar row.

## Shortcuts

All shortcuts are configurable via the `keys` section of the config file
(above); the table lists the defaults. With a terminal focused:

| Action             | Windows / Linux            | macOS         |
| ------------------ | -------------------------- | ------------- |
| Copy selection     | `Ctrl+Shift+C`             | `Cmd+C`       |
| Paste              | `Ctrl+V` / `Ctrl+Shift+V`  | `Cmd+V`       |
| Next / prev tab    | `Ctrl+Tab` / `Ctrl+Shift+Tab` | (same)     |
| Jump to tab 1–9    | `Ctrl+1` … `Ctrl+9`        | `Cmd+1` … `Cmd+9` |
| Close tab          | `Ctrl+W`                   | `Cmd+W`       |
| Focus search       | `Ctrl+F`                   | `Cmd+F`       |
| Interrupt (SIGINT) | `Ctrl+C`                   | `Ctrl+C`      |

Jump-to-tab (`Ctrl`/`Cmd`+`1`–`9`) is matched by physical key position, so it
lands on the same number-row keys on every layout — including AZERTY and QWERTZ,
where those keys produce `&`, `é`, … without Shift.

Dragging a selection with the mouse also copies it on release, and the wheel
scrolls back through history. In the sidebar, click a project or session to
open it; a tab's `×` also closes it.

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
