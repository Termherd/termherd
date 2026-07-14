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
[Releases](https://github.com/Termherd/termherd/releases) page. Pick the
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
  https://github.com/Termherd/termherd/releases/latest/download/termherd-installer.sh | sh
```

```powershell
# Windows
powershell -c "irm https://github.com/Termherd/termherd/releases/latest/download/termherd-installer.ps1 | iex"
```

### Verify a Linux download

Linux release binaries carry a sigstore *keyless* build-provenance attestation
(no signing key — the signer is the release workflow, via GitHub OIDC, logged
in the public Rekor transparency log). Verify a download with the `gh` CLI:

```bash
gh attestation verify termherd-x86_64-unknown-linux-gnu.tar.xz \
  --repo Termherd/termherd
```

A successful check proves both integrity and that the artifact was built by
this repository's CI. A `SHA256SUMS` file is also attached to each release.

## Run from source

```bash
cargo run -p termherd-app
```

## Configuration

Optional user settings live in `~/.termherd/settings.json` (on Windows,
`%USERPROFILE%\.termherd\settings.json`). The file is read at startup; if it
is missing or invalid, TermHerd falls back to defaults rather than refusing
to start — out-of-range values clamp, and a single bad value (a typo'd
colour, an unknown key action) degrades alone with a logged warning instead
of resetting the rest of the file. There is no in-app settings panel yet —
edit the file and restart.

The annotated reference template is
[`docs/settings.example.jsonc`](docs/settings.example.jsonc): every option
that exists today, with its default value and what it does. Copy the blocks
you want and strip the comments (the real file is strict JSON). In short:

- `shell` — program + args launched for each session (default: the platform
  login shell).
- `theme` — `"dark"` (default) or `"light"` GUI chrome; the terminal grid
  keeps its own colours.
- `close` — per-action close confirmation (`tab`, `app`): always, only while
  a foreground process runs (default), or never.
- `terminal` — base `font_size` (the zoom shortcuts step from it) and grid
  `colors` (a named scheme — Solarized / Gruvbox, dark or light — plus
  per-slot overrides).
- `sidebar` — sessions listed per project before the tail folds behind an
  expander (`0` shows all).
- `record` — the GIF screencast budget (fps, duration cap, frame scale).
- `keys` — keyboard overrides, one chord or a list per action; the full
  action vocabulary and its default chords are listed in the template.

The same options are also readable and writable from inside a Claude session
via the MCP control surface (below).

Window size and position persist separately to `~/.termherd/window.json` (a
position left off every connected monitor — e.g. on a screen since unplugged —
is dropped so the window re-centers instead of opening out of reach), and
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
| Scroll top/bottom  | `Ctrl+Up` / `Ctrl+Down`    | `Cmd+Up` / `Cmd+Down` |
| New shell here     | `Ctrl+T`                   | `Cmd+T`       |
| New Claude here    | `Ctrl+Alt+T`               | `Cmd+Alt+T`   |
| Reopen closed tab  | `Ctrl+Shift+T`             | `Cmd+Shift+T` |
| Close tab / pane   | `Ctrl+W`                   | `Cmd+W`       |
| Split vert. / horiz. | `Ctrl+D` / `Ctrl+Shift+D` | `Cmd+D` / `Cmd+Shift+D` |
| Focus pane         | `Ctrl+Shift+←↑↓→`          | `Cmd+Shift+←↑↓→` |
| Zoom in / out / reset | `Ctrl` + `+` / `-` / `0` | `Cmd` + `+` / `-` / `0` |
| Focus search       | `Ctrl+F`                   | `Cmd+F`       |
| Capture state dump | `Ctrl+Shift+S`             | `Cmd+Shift+S` |
| Record GIF (start/stop) | `Ctrl+Shift+R`        | `Cmd+Shift+R` |
| Interrupt (SIGINT) | `Ctrl+C`                   | `Ctrl+C`      |

Jump-to-tab (`Ctrl`/`Cmd`+`1`–`9`) is matched by physical key position, so it
lands on the same number-row keys on every layout — including AZERTY and QWERTZ,
where those keys produce `&`, `é`, … without Shift.

Dragging a selection with the mouse also copies it on release, and the wheel
scrolls back through history. In the sidebar, click a project or session to
open it; a tab's `×` also closes it. Hovering a tab shows the session's fuller
description (the same card the sidebar shows).

## MCP control surface (experimental)

`termherd-mcp` is a small [MCP](https://modelcontextprotocol.io) server that
exposes termherd's own configuration to a Claude session, so you can ask "what
can I configure here?" — or "switch me to a light theme" — from inside the
conversation termherd already hosts. It exposes two tools, `list_options`
(read) and `set_option` (write), plus the option **schema** as a resource, all
reflecting `~/.termherd/settings.json`. Workspace orchestration (open session,
split, focus, …) is a planned follow-up (`F-mcp-control-surface`, [#90]).

It speaks JSON-RPC over stdio. Register it with Claude Code by adding it to your
`mcpServers` config (point `command` at the built binary):

```json
{
  "mcpServers": {
    "termherd": { "command": "/path/to/termherd-mcp" }
  }
}
```

Build the binary with `cargo build -p termherd-mcp` (it lands in `target/`).

[#90]: https://github.com/Termherd/termherd/issues/90

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
