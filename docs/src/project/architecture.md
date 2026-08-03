# Architecture at a glance

TermHerd is a replatform of an Electron session manager, and the rewrite is
scoped by the defects it must fix **by construction** — a god object, races,
silent catches, and an untestable design. Everything below is downstream of
that.

The authoritative documents are
[`docs/PRD.md`](https://github.com/Termherd/termherd/blob/main/docs/PRD.md) and
[`docs/ARCHITECTURE.md`](https://github.com/Termherd/termherd/blob/main/docs/ARCHITECTURE.md).

## The dependency rule

A hexagonal workspace. Adapters depend on the core; the core depends on no
adapter.

```text
   app  ──────►  core  ◄──────  adapters
   (iced GUI)      │           (scan, pty)
                   ▼
                claude
             (pure codec)
```

| Crate | Is | Depends on |
| --- | --- | --- |
| `core` | the domain: headless `App` state machine, the tab/pane tree, the keymap, the port traits | `claude` only |
| `claude` | a pure codec for the Claude CLI's own formats — path encode/derive, JSONL digest, OSC decode | nothing |
| `app` | the iced GUI shell; builds the adapters in `main()` and injects them; owns the one effect executor and the MCP control surface | `core` + adapters |
| `scan` | filesystem discovery — walks `~/.claude/projects` | `core` |
| `pty` | the terminal adapter (`portable-pty` + `alacritty_terminal`) | `core` |
| `mcp` | the stdio settings server | `core` |

When adding code the question is *which crate does this belong in?* If the
answer is "the core should call this adapter directly", the answer is wrong —
add a port trait and have the adapter implement it.

## The headless core

```rust,ignore
core::App::apply(Event) -> Vec<Effect>
```

Elm-style, and **pure**: no I/O, no clock, no panic. The GUI translates user
gestures into `Event`s and performs the returned `Effect`s. Everything testable
lives behind `apply` — which is also why the [MCP surface](../mcp/index.md)
adds no new mutation path: every tool goes through an `Event` the keyboard
already used.

The pane tree is pure data, exhaustively unit-testable. Its mutators return
`Option<()>` rather than panicking when an invariant is violated — a broken
invariant surfaces as `None`/`Err`, never an `unwrap`.

## Concurrency

One tokio runtime, **actor per session**: each session is owned by a task
holding its PTY handle and terminal grid, reachable only by channel. There is
no shared `&mut Session`. The GUI thread owns the `App` and applies events
single-threaded.

That structure is the fix for the session-id race the predecessor had, not a
mitigation of it.

## The quality bar

Non-negotiable, and CI-enforced:

- `unwrap`, `expect` and `panic` are **clippy-denied** in `core` and `claude`.
  Production paths return typed errors.
- `unsafe_code` is **denied workspace-wide**. The one sanctioned exception is
  the macOS AppKit FFI for the Cmd+Q quit path: a `cfg`-gated module with a
  `SAFETY:` note on every block.
- **No global mutable state** — no `static mut`, no `lazy_static`, no
  require-time singletons. Dependencies are built in `main()` and injected.
- **One logging stack**: `tracing`. No `println!` outside tests.
- **Function length is gated** (`clippy::too_many_lines`, threshold 150). A
  function that exceeds it on purpose carries a local allow with a rationale.
- Every gate runs on every PR — formatting, clippy with warnings denied, the
  test suite, dependency licensing, unused dependencies, the crate dependency
  rule, intra-crate module boundaries, and markdown lint. Portable crates are
  additionally **built and tested on Windows on every PR**.

The full CI reference is
[`docs/CI.md`](https://github.com/Termherd/termherd/blob/main/docs/CI.md).

## Toolchain

Pinned to **Rust 1.95.0**, edition 2024, via `rust-toolchain.toml`.
