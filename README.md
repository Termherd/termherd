# termherd

> A Rust replatform experiment for a Claude Code session
> workspace. Native, terminal-multiplexer-style (tabs + splits + keyboard
> driven), with the quality bar the predecessor lacked.

This is an early scaffold. Status, scope, and design live in:

- [`docs/PRD.md`](docs/PRD.md) — Product Requirements Document
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — Architecture
- [`ROADMAP.md`](ROADMAP.md) — feature buckets (MoSCoW)
- [`CHANGELOG.md`](CHANGELOG.md)

## Run

```bash
cargo run -p termherd-app
```

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
