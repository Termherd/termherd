# AGENTS.md

## What this is

`termherd` is a Rust replatform of an Electron Claude Code session
manager. The product is a **terminal workspace for Claude Code sessions** —
browse, launch, arrange (tabs + splits), monitor, search — driven from the
keyboard, on macOS, Windows, and Linux (all three first-class). The restart
exists to fix four quality gaps
(god-object, races, silent catches, untestable design) **by construction**.

Authoritative design lives in `docs/PRD.md` and `docs/ARCHITECTURE.md`. Read
them before any non-trivial work — the constraints below are downstream of
them.

## Commands

```bash
cargo run -p termherd-app          # run the binary (M0: tracing + single-instance stub)
cargo test --workspace             # all tests
cargo test -p termherd-core        # tests for one crate
cargo test -p termherd-core workspace::tests::split_wraps_leaf  # one test by path

# CI gates — mirror locally before pushing (CI runs all of these and they are blocking)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace             # CI uses `cargo nextest run --workspace`
cargo deny check                   # if cargo-deny installed
cargo machete                      # unused deps; if cargo-machete installed
just check-deps                    # hexagonal crate dependency rule (deps point inward)
just check-arch                    # intra-crate module boundaries + OS-cfg containment (+ length report)

# Markdown is also gated in CI (ROADMAP.md included — a lint failure there is a
# roadmark bug, not something to fix by editing the artifact)
markdownlint-cli2                  # uses .markdownlint-cli2.jsonc
just roadmap                       # recompile ROADMAP.md from .roadmap/, then validate

# Planning hygiene — not a CI gate (needs a `project`-scoped token)
just board-check                   # board/issue drift (0 clean · 1 drift · 2 unchecked)
```

Toolchain is pinned to **Rust 1.95.0 / edition 2024** via `rust-toolchain.toml`
(Q10) — do not bump without updating the pin.

CI runs each gate **only when its file category changed** (a `changes` job with
`dorny/paths-filter`): a docs-only PR skips every Rust job, a pure-`.rs` change
skips the dependency-metadata jobs. All gates fan into one required check,
`ci-success`, which treats path-skipped jobs as passing — so `main` branch
protection pins that single check. Gate any new job on its category; never make
a path-filtered job a *required* check directly.

One job runs somewhere other than ubuntu: **`portable`** builds and tests every
crate but `app` on **Windows, on every PR**. The wider `cross-os` matrix still
runs only after merge, which is how a Windows-only defect once landed green and
surfaced three merges later against an unrelated push. So: a change to `core`,
`claude`, `pty`, `scan` or `mcp` is checked on Windows before it merges, and
there is no local mirror for it — `cargo check --target` catches compile
breakage, never behaviour.

Full CI reference — every gate, its goal, when it runs, how to mirror it, and
the sanctioned exceptions — lives in [`docs/CI.md`](docs/CI.md).

### Running & observing a build

Some behaviour is GUI/OS-level and **cannot be exercised by a headless test**
— the macOS Cmd+Q quit-confirm flow, window placement, the PTY canvas. Verify
those by running the app and reading its `tracing` output:

```bash
# `tracing` is the only observation channel — there is no `println!`. Raise the
# level with RUST_LOG (default is `info,…`, see `DEFAULT_FILTER` in main.rs).
RUST_LOG=info cargo run -p termherd-app

# Add log lines at the seam you're verifying (info!/warn!, never println!), run,
# and grep the output for them — e.g. the quit path logs `request_quit`'s branch
# and the macOS menu repoint.
```

The app is **single-instance** (an flock at `std::env::temp_dir()/…`). To run a
build *alongside* one that already holds the lock — common, since a dev/agent
session often runs *inside* a release `TermHerd.app` you can't quit — point the
new process at a throwaway temp dir so its lock path differs:

```bash
TMPDIR=$(mktemp -d) RUST_LOG=info cargo run -p termherd-app   # second instance
```

`temp_dir()` honours `$TMPDIR`, so both run. Launch detached when you need to
keep interacting with the original window (e.g. to compare quit behaviour).

**`cargo test` used to need `env -u ZDOTDIR` when run from inside a termherd
shell — it no longer does.** That shell exports
`ZDOTDIR=$TMPDIR/termherd-shell-<id>`, which a nested termherd read as the
user's own home: with the session ids restarting at 1 each run, the `.zshenv`
it then generated sourced *itself* (`job table full or recursion limit
exceeded`) and the shell never reached a prompt. It surfaced as
`typing_exit_into_a_real_shell_closes_the_tab_end_to_end` timing out and read
as a regression in whatever you had just changed. Fixed in #244: a directory
named `termherd-shell-*` is never replayed from, and the zsh recipe exports
`TERMHERD_ORIG_ZDOTDIR` so the real home crosses any nesting depth.

To reproduce that class of bug deliberately, poison the variable on the command
line — a test cannot do it for itself, since `std::env::set_var` is `unsafe` in
edition 2024 and the workspace denies `unsafe_code`:

```bash
env ZDOTDIR="$TMPDIR/termherd-shell-1" cargo test -p termherd-app \
  typing_exit_into_a_real_shell
```

### Capturing state for the AI dev loop (#108)

Press **⌘⇧S** (macOS) / **Ctrl+Shift+S** (rebindable as `capture`) to dump the
running app's state for an AI assistant to read — rung 0+1 of `F-capture`. Each
press writes a timestamped pair to `~/.termherd/captures/`:

- `capture-<ts>.json` — a diffable state dump of the whole workspace: focus,
  resolved config, the sidebar, every tab with its panes (each pane's stable
  handle, kind, cwd, status), and the focused terminal's visible text. No vision
  needed.
- `capture-<ts>.png` — the real window pixels (iced `window::screenshot`), for
  render / colour / glyph bugs the text dump can't show.

The dump **is** the `WorkspaceSnapshot` the MCP `snapshot` tool reports, under a
fixed full filter (`SnapshotFilter::capture()`) — one model, two readers, so a
field never means one thing on disk and another on the wire.

`<ts>` is a UTC `YYYYMMDD-HHMMSS-mmm` stamp, so the **latest capture is the
highest-named pair** — an AI finds it by sorting the directory. Capture stays
pure in `core` (`Event::Capture(SnapshotInputs)` →
`Effect::Capture(WorkspaceSnapshot)`); all I/O — the clock, JSON/PNG encoding,
the files — lives in the `app` adapter (`crates/app/src/capture.rs`), which
shares its wire form with the MCP handler (`crates/app/src/snapshot_dto.rs`).

For motion (rung 2, #124), press **⌘⇧R** / **Ctrl+Shift+R** (rebindable as
`toggle-record`) to start a **GIF screencast**; press again to stop, or let it
auto-stop at the cap (default 8 fps / 30 s / 0.5× scale, set under a `record`
block in `settings.json` — #127). It writes `capture-<ts>.gif` to the same dir.
Same hexagonal split: `core` owns the
idle→recording state machine (frames are the time proxy — no clock), and the
`gif` encoder runs on a dedicated thread in `app` (`crates/app/src/record.rs`)
so the UI — and the recording — stay smooth.

### Driving termherd over MCP (#90)

A Claude session **launched from termherd** gets an in-process MCP server wired
into its `mcpServers` at spawn (loopback, per-session token) — so it can read
and drive the workspace it runs in. This is the richer sibling of the capture
dump above: same `WorkspaceSnapshot` model, live instead of a file.

**Settled.** Eight slices shipped: `list_sessions` + `snapshot` (perception),
`open_session` / `split_pane` / `focus_pane` / `rename_tab` / `close_pane` /
`run_in_session` (action), `wait_for_status` + `read_terminal`
(synchronisation), `screenshot` (pixels), `press_keys` + `run_action`
(the app's own keyboard). The loop they exist to serve is
**act → wait → observe**: `run_in_session` returns immediately, so synchronise
with `wait_for_status` and then `read_terminal`. Do **not** poll `snapshot` in
a loop — it races the transition you are watching for, which is why the wait
rung exists.

**That loop did not run until #236, and now does.** Every session used to sit
on `starting`, so `wait_for_status` only ever settled by timing out. Two
independent holes, both closed: a plain shell spoke none of the Claude OSC
dialect the status fold understood, and now runs with an injected OSC 133
shell-integration snippet — with the PTY's foreground process group standing in
where the snippet cannot apply, and nothing at all under ConPTY; and a
`CLAUDE_CODE_DISABLE_TERMINAL_TITLE` in the user's own `~/.claude/settings.json`
silenced the Claude channel outright, which a private `--settings` overlay on
the launch line now outranks. That overlay is why termherd needs **Claude Code
1.0.61 or newer** — an older CLI rejects the flag and the launch fails.

The same stuck status also kept a close confirmation from arming for a *shell*
(`has_running_process` needs `Busy` or `Attention`); that follows from the fix
rather than needing one of its own, so a shell running a command now confirms
as a Claude session always did.

`screenshot` is the pixel companion to the text `snapshot`, for the render,
colour and glyph questions text cannot answer. Reach for it *last*: a
default-bound window is ~200 kB of PNG and a third more again as base64, where
a `snapshot` is a few hundred bytes. Two bounds keep that honest — `max_width`
(default 1200) and a total-pixel ceiling for tall windows a width alone never
reaches — and a window smaller than them is never upscaled. Shrinking averages
the covered pixels rather than picking the nearest one, so terminal glyphs stay
legible at the ~0.4× a retina window gets reduced by; that legibility is what
the bytes buy. Lower `max_width` when a coarse view will do. A headless run
has no window and says so as a tool-level error; the text reads keep working.

`press_keys` and `run_action` drive termherd's **own interface**, not a terminal
— raw keys *into* a session stay `run_in_session`'s job. They are the same
dispatch asked two ways: `press_keys` takes chords in `settings.json` syntax and
resolves them through the **live** keymap, so it tests the *binding* (including
the user's overrides); `run_action` takes the kebab-case action names (the
catalogue the stdio server publishes at `termherd://keys/schema`) and skips the
keymap, so it tests the *behaviour* and survives a rebind.

A chord is dispatched as a **synthesised key event** fed to `Shell::on_key` —
the whole ladder, not just `Keymap::lookup`. That is what makes `escape` and
`enter` reachable: they are *overlay* keys bound to no action, so an agent that
armed a close-confirmation would otherwise have no way to answer it and would
park the app until a human intervened. The corollary is that an open overlay
consumes an MCP press exactly as it consumes a keypress — reported per press,
naming the prompt in the way, so a caller learns why its chord did nothing.
`run_action` is gated on the same ladder, deliberately: neither tool may reach a
state the keyboard cannot.

Each press answers with what the ladder did — `ran` (with the action's name),
`inert` (nothing happened, with a `reason`), `overlay` (which prompt ate it),
`typed` (bound to nothing, so it reached the focused terminal), `unbound`
(nothing claimed it) — plus the resulting `focused_handle`. A malformed chord or
unknown action name rejects the **whole** call before anything applies: half an
applied sequence is worse than none, since the caller cannot tell how far it
got.

`inert` carries its `reason` because the two kinds of nothing call for opposite
responses: `no-surface` means the action is wired to nothing, so retrying is
pointless (`open-new-session` is the one), while `no-context` means a
precondition was absent — nothing focused to derive a repo from, no closed tab
to reopen, nothing to scroll, nothing selected to copy — which the caller can go
and *create* before trying again. Seven handlers can refuse that way, and each
says so at its own refusal (they return `Option`), so no predicate here has to
re-derive the list.

The line is whether the shell refused, **not** whether the effect was
interesting: `activate-tab-9` on a single-tab workspace reports `ran`, because
`core` applied the event and absorbed it. Collapsing the two would make the
distinction useless.

Sessions are addressed by a stable `handle` (the runtime `SessionId`), never
the Claude `resume_id`, which re-keys on a fork / plan-accept (Q6). Every call
is `tokio::timeout`-bounded in `BridgeHandle::call` (Q7).

Where it lives: tools in `crates/app/src/mcp/handler.rs`, transport in
`shell::bridge`, and the shell's answers in `shell::serve` — the one place an
external caller meets `core::App`. `core` has no MCP awareness at all; every
mutation goes through an existing `Event`. The keyboard rung adds
`shell::orchestrate::perform_presses` beside the action path, `input::event_of`
(the inverse of `chord_of`), and `routing::KeyboardOwner` / `KeyVerdict` — the
overlay ladder and its outcome named once, since three readers consult them.

**Still open.** `F-mcp-agent-loop` (#196 — the composed prompt→wait→read in one
round trip), the last child of the #90 epic. With `screenshot` and the keyboard
tools the capability is otherwise whole in three parts: drive the UI, see the
pixels, read the terminal — what lets an agent verify a gesture fix instead of
only proposing it. #196 *composes* the wait, which #236 had to fix first —
building it on a synchronisation that never fired would have been building on
sand, and that ordering constraint is now discharged.

**Every overlay can now be left from the keyboard** (#237). An open sidebar
session-rename used to swallow every key including `escape`, parking the whole
control surface until a human cleared it with the mouse — the exact failure the
synthesised-key-event design exists to prevent, prevented for the confirmation
prompts and not for that one. `escape` now abandons the rename, matching the
blur it stands in for.

The interesting part is what closed it for good: the fix carries a sweep driven
off `KeyboardOwner::ALL` rather than a test per prompt, and that sweep found a
*second* offender nobody had reported — the doc editor, closable only by its own
button. A rung added without an exit fails there now. `escape` there closes
exactly as the button does, discarding unsaved edits: a known trade, since a
stricter gesture for the key alone would leave the button's identical hole
standing. Losing a modified doc silently is a defect on both paths and is #248.

The sweep is the general form of the "test that claims to be exhaustive" rule
below: it is honest because `arm_overlay`'s `match` is compiler-checked, so a
new variant cannot slip past it. (**#236**, the sibling defect that left every
session on `starting`, is fixed — see the wait rung above.)

One gap remains and is **not** a variant of this: `enter` reaches neither rename
through MCP, because both commit via the widget's `on_submit`, which a
synthesised key event never touches. That is a missing capability rather than a
parked surface — a caller can always `escape` and start over — so it is tracked
separately, as #246.

**Looks like a contradiction, is not.** `docs/ARCHITECTURE.md` §15 lists an
`mcp` crate as *deferred (Unsure)*. That is a **different feature** —
`F-mcp-ide-bridge`, termherd as an MCP *client* of Claude's IDE bridge — and it
really is unbuilt. The surface described here runs the other way round
(termherd is the server) and lives in `app`, not in a crate of its own.

## Architecture — the dependency rule

Hexagonal workspace. The single most important invariant:

```text
app  ──►  core  ◄──  adapters          (adapters depend on core, never reverse)
           │
           ▼
         claude   (pure codec; no I/O)
```

- `crates/core` — domain, headless `App` state machine, `Workspace` (pane
  tree + tabs), keymap, port traits. **Depends only on `claude`.** No I/O, no
  globals, no `unwrap`/`expect`/`panic` (these are clippy-denied here, see
  `crates/core/Cargo.toml`).
- `crates/claude` — pure Claude CLI format codec (path encode/derive, JSONL
  digest, OSC decode). Same strict lint profile as `core`.
- `crates/app` — iced GUI shell. Constructs the adapters in `main()` and
  injects them into `core::App`; owns the one effect executor
  (`shell::effects`) and the MCP control surface (`app::mcp` + `shell::bridge`
  / `shell::serve`).
- `crates/scan` — filesystem discovery adapter (walks `~/.claude/projects`
  via the `claude` codec; implements `core::ports::ProjectScanner`).
- `crates/pty` — terminal adapter (`portable-pty` + `alacritty_terminal`);
  implements `core::ports::PtyHost`.
- `store` (Should, PRD rev. 4) is the one adapter still unbuilt. The **MCP
  control surface shipped** as a module inside `app`, not as its own crate —
  it is a bridge into the shell, not a port `core` calls out through. The
  separate `mcp` *crate* sketched in `docs/ARCHITECTURE.md` §15 is a different
  feature (`F-mcp-ide-bridge`: termherd as an MCP **client** of Claude's IDE
  bridge), still unbuilt.

When adding code, ask: *which crate does this belong in?* If the answer is
"`core` should call this adapter directly," the answer is wrong — add a port
trait in `core::ports` and have the adapter implement it.

## The headless core (where logic lives)

`core::App::apply(Event) -> Vec<Effect>` is the Elm-style heart of the system
(`crates/core/src/app.rs`). It is **pure**: no I/O, no clock, no panic. The
GUI translates user actions into `Event`s and performs the returned
`Effect`s. Everything testable lives behind `apply`.

`Workspace` (`crates/core/src/workspace.rs`) is the tab/split pane tree — pure
data, exhaustively unit-testable. The focus path is a `Vec<Branch>` from the
root; mutators (`open`, `split`, …) return `Option<()>` rather than panicking
when invariants are violated. Follow that pattern: surface broken invariants
as `None`/`Err`, never `unwrap`.

## Concurrency model (when adapters arrive)

One tokio runtime, **actor-per-session**: each session is owned by a task
holding its PTY handle and terminal grid. Other parts of the system talk to
it only via channels. There is no shared `&mut Session`. The GUI thread owns
`core::App` and applies events single-threaded. This is the structural fix for
the `realSessionId` race (Q6 in `docs/PRD.md` §4) — keep it.

## Quality bar — non-negotiable

Each rule below is tied to a Q-row in `docs/PRD.md` §4 (the reason the rewrite
exists). Do not relax them locally.

- **`core` and `claude`**: clippy denies `unwrap_used`, `expect_used`, `panic`.
  Tests may use them (`clippy.toml` allows it in tests). Production paths
  return typed errors (`thiserror`).
- **No global mutable state.** No `static mut`, no `lazy_static`, no
  require-time singletons. Construct dependencies in `main()` and inject.
- **One logging stack:** `tracing`. No `println!` outside tests.
- **`unsafe_code = "deny"`** workspace-wide. The lone sanctioned exception is
  `crates/app/src/macos.rs` (AppKit FFI for the Cmd+Q quit path): a `#![cfg(…)]`
  module with a module-scoped `#![allow(unsafe_code)]` and a `// SAFETY:` note
  on every block. Any further exception needs the same — OS-FFI that can't be
  expressed safely, quarantined in its own `cfg`-gated module — not a relaxation
  scattered through otherwise-safe code.
- **A `cfg`-gated API is not compiled by the PR gate — cross-check it
  yourself.** The `cross-os` job does not run on pull requests, so
  `cargo check --target x86_64-pc-windows-msvc` is the *only* thing standing
  between a Unix-only call and a broken packaging build. `#236` called
  `MasterPty::process_group_leader`, which `portable-pty` declares under
  `#[cfg(unix)]`, from an ungated file: green on every PR check, and it would
  not compile for Windows. Worse, the code's own doc-comment asserted the
  opposite ("`portable_pty` reports none on Windows") — true of the *value*,
  but the method is absent, so there was nothing to return none. Whenever a
  diff touches a dependency's platform-conditional surface, add the target once
  (`rustup target add x86_64-pc-windows-msvc`) and check both ways: the change
  compiles, and reverting it fails.
  **A test that needs an OS-only API belongs in `tests/`, not in the
  containment allow-list.** `check-os-cfg-containment.sh` scans `*/src/**`
  only, so an integration test may use `std::os::unix::fs` freely — whereas
  adding the adapter's own source file to the allow-list to accommodate one
  test would licence OS-conditional *production* code there unnoticed, which is
  what the quarantine exists to prevent. Budget for one surprise on the way:
  `clippy.toml`'s `allow-expect-in-tests` recognises `#[cfg(test)]` items, not
  a `tests/` binary, so such a file needs its own `#![allow(clippy::…)]` with a
  reason.
- **Cross-compiling is not the whole hazard — code that *runs* differently per
  host is worse, because no `cargo check` catches it.** `#239`'s shell
  integration built the replayed startup path with `Path::join`, which writes
  the host's separator into what is a line of **POSIX shell**: correct on Unix,
  a backslash escape on Windows, so the user's own rc was silently never
  sourced. It compiled everywhere and three tests asserted the Unix spelling.
  Because `cross-os` skips PRs *and* is path-filtered on `rust`, two docs-only
  merges went by before the next `.rs` push to `main` surfaced it — the break
  was attributed to that push, not to the one that caused it. The rule:
  **a string destined for another grammar takes that grammar's separators, not
  `std::path`'s.** When a `Path` is being rendered into a script, a URL, or a
  wire format, join by hand. And when `main` goes red on a Rust push, check
  whether the previous Rust push is the real author before reading the diff in
  front of you.
- **Async work inside an `iced::Task` is not on tokio — `spawn_blocking` there
  is fatal, and silently so.** `iced` is built here without its `tokio`
  feature, so `Task::perform` polls on the `futures` thread pool, and `main`
  hosts the MCP runtime with `block_on` rather than entering it. A
  `tokio::task::spawn_blocking` in that future calls `Handle::current()` and
  panics. The panic is invisible twice over: the effect it belonged to simply
  never answers, *and* the unwind takes a pool worker with it — the pool does
  not respawn one, so after roughly `num_cpus` occurrences **every** `Task` in
  the app stops completing (scans, captures, the PNG encode, the bridge). The
  clickable-path resolution (#252) was written this way and reached review with
  839 tests green, none of which could have caught it: no test polls a returned
  `Task`. Do blocking work directly in the `async` block, the way
  `scanner.scan()`, `docs::discover` and `capture::write_png` already do; reach
  for a `std::thread` + channel when it must outlive the task. The rule is
  about *where the future runs*, not about which file it lives in: a `tokio::`
  call is fine in code the runtime spawned (`shell::bridge`, `app::mcp`) and in
  a `#[tokio::test]`, and wrong in anything handed to `Task::perform`.
- **Function length is gated.** `clippy::too_many_lines` (threshold 150 in
  `clippy.toml`) fails CI on over-long functions — a proxy for local
  complexity. A function that exceeds it on purpose (a flat dispatcher / layout
  builder) carries a local `#[allow(clippy::too_many_lines)]` with a rationale,
  never a relaxed global threshold.
- **An invariant expressed twice will drift — extract the predicate.** Two
  call sites deciding "has this settled?" with hand-written conditions is a
  bug waiting on the first edit that touches one of them. It bit the
  `wait_for_status` rung: one site treated a session exit as settling a wait,
  the other only compared against the requested statuses, so a wait placed
  after a crash parked until the caller's timeout. Both now go through one
  `settles()` predicate. A doc-comment asserting the rule is *not* enforcement
  — the comment describing the correct behaviour sat directly above the code
  that broke it.
- **A guard is unreachable and goes, or reachable and gets a test — there is
  no third state.** Defensive arithmetic nobody can trigger is not free: it
  reads as a live case to the next reader, and no test can pin it. Mutation
  testing finds these by construction, because a mutant of dead code changes
  nothing observable and survives. In `image::resample_box` the clamps were
  provably unreachable under its shrink-only precondition and went, while
  `count > 0` — all that stands between a box with no readable pixel and a
  divide by zero — was reachable and earned a truncated-buffer test. Both
  survivors looked like missing assertions and were really design smells.
- **A test that claims to be exhaustive is worse than no test when it is not.**
  A hand-written list asserting "this *set* is the contract" reads as a
  guarantee, so nobody re-derives it — where an absent test at least leaves the
  reader suspicious. `every_action_that_can_refuse_reports_which_kind_of_nothing`
  said so in its own comment and listed five of the seven actions that refuse;
  the two it missed (`copy` with nothing selected, `close-focused` on an empty
  workspace) went on reporting success at doing nothing, which is the exact
  failure the test existed to kill. Either derive the list from the same source
  the code uses, or make each case fail loudly on its own — never assert
  completeness in prose above an enumeration a human typed.

## Conventions

- Coding standards (Tidy First, CUPID & YAGNI, TDD + Reflect, Clean Code) live
  in [`CODING_STANDARDS.md`](CODING_STANDARDS.md). This file (AGENTS.md) takes
  precedence where they collide.
- Markdown prose: 80-col wrap (tables / code blocks exempt, see
  `.markdownlint-cli2.jsonc`).
- Commit messages: no "Claude" signature (per global user instruction).
- No issue numbers (`#NN`) in code comments, doc-comments, or test names —
  git history already links code to its issue, and an in-code `#NN` rots when
  issues are renumbered or migrated. Cite issues in commit/PR bodies and
  `ROADMAP.md`/PRD prose instead. Full rationale in
  [`CONTRIBUTING.md`](CONTRIBUTING.md).
- A reference code in a comment must be resolvable without external context:
  either name the rule in plain language, or use a code **whose source this
  file records.** The one sanctioned code is **`FRn` = the numbered Functional
  Requirements in [`docs/PRD.md`](docs/PRD.md) (§Functional requirements)** —
  e.g. `FR4` is the embedded-terminal requirement, `FR6` splits. Do not coin
  other bare abbreviations; a lone `FR4` is only readable because of this line.
- Status of every feature is tracked in `.roadmap/features/*.md` (MoSCoW from
  PRD §5), and compiled into `ROADMAP.md`. Check the `status =` line — or the
  glyph in the generated catalog — before assuming something is built.
- **`ROADMAP.md` is generated. Never edit it.** Edit the feature file, then
  run `just roadmap` and commit both. CI rebuilds and diffs.

## How we track work

Three layers, each owning one thing — no item lives fully in two places:

- **`.roadmap/` (+ `docs/PRD.md`)** — the *what* and *why*: features, MoSCoW
  bucket, shipped history with rationale, and design-first epics not yet scoped
  enough to act on (e.g. `F-i18n`, `F-favorites`). Source of truth for whether
  a feature exists. One feature is one file, so two people never edit the same
  line; `ROADMAP.md` is compiled from it by [roadmark](https://github.com/bastien-gallay/roadmark)
  and is a build artifact — read it, don't write it.
- **GitHub issues** — the *unit of work*: actionable, scoped tickets. Each
  carries a native **issue type** (`Feature` / `Bug` / `Task`) and one or more
  **`area:*`** labels; `os:*` and `needs-design` are modifiers on top.
- **[Project board](https://github.com/orgs/Termherd/projects/1)** — canonical
  for **priority and order**, held as sortable single-select fields: `Horizon`
  (Now / Next / Later / Parked / Shipped), `Class`, `Effort`, `Severity` (see
  **Priority scheme** below). Edit these there, visually — not in a file.

### Priority scheme

Priority is **two orthogonal axes**, not one `Pn` number (which conflated
impact, urgency, and cost — the `P0`–`P3` labels were retired 2026-07-26).

- **Class** — the *kind of leverage* a Feature delivers: **⚡ Differentiator**
  (the thesis edge) · **🔑 Enabler** (unblocks other work) · **📐 Table-stakes**
  (expected of any such tool; its absence is a wart) · **✨ Polish**
  (ergonomics) · **🎲 Bet** (uncertain — prototype to learn).
- **Effort** — **S / M / L**.
- **Ordering rule**: within a Horizon, **small Differentiators & Enablers
  first**; a **Bet gets a timeboxed probe, not a full build**.

Bugs are **not a Class** — they restore a contract, they don't add leverage.
A `Bug` carries a **Severity** instead (🔴 Critical / 🟠 Major / 🟡 Minor) and
jumps the queue on severity × blast-radius, off the leverage map. A `Task`
(packaging, tooling, chores) carries neither.

So: **Class / Effort / Severity / Horizon = board fields · Type = the native
issue type · Area = `area:*` labels · everything narrative = `.roadmap/`.**

`.roadmap/config.toml` declares no `horizon`, `class` or `effort` on purpose —
those are the board's, and a second copy here would be a second thing to keep
true. The roadmap's own `area` axis is a separate, coarser vocabulary from the
`area:*` labels: it answers "what part of termherd does this change", one line
in a frontmatter, not a label to triage by.

The one rule that keeps it sane: an epic **graduates from the roadmap to an
issue only when it's scoped enough to do.** A design-first item lives only in
the roadmap until then; once filed as an issue it appears on the board. Flip
the feature's `status` to `done` when its issues close.

Two corollaries that keep the layers in sync (both contributors work from
issues, so a scoped roadmap item with no issue is invisible):

- **When an epic graduates, link it both ways.** Open the issue *and* add its
  `#number` to the feature's body. Shipped entries already cite their issues;
  do the same for open ones.
- **A cross-reference between features is a link, not a name.** Write
  `[F-mcp-keys](#f-mcp-keys)`, never a bare `F-mcp-keys`: `roadmark validate`
  fails on a link to an id nothing declares and `roadmark rename` rewrites it,
  while a bare mention only earns a warning. That is why the nine MCP rungs are
  nine files rather than sub-bullets — nesting them would have put ten
  unverifiable ids in one body.
- **Design a backlog epic before filing it.** Run `/feature-torture` on a
  design-first item to reach a verdict (ship / reshape / park / split / kill);
  file issues only for the slices that come out scoped. The report lands in
  `.personal/feature-torture/reports/<F-id>.md`; cite it in the ROADMAP entry.
  Items that stay design-first (e.g. `F-keymap-per-command`) live only in the
  roadmap until their blocking design is resolved.
- **`just board-check` reports board/issue drift** — an open issue the board
  never classified (filed, then invisible to every view), and an item whose
  `Status` and `Horizon` disagree about having shipped. That second one is
  structural, not hypothetical: closing an issue flips `Status` to Done and
  leaves `Horizon` alone, so a landed feature keeps reading as *Next* until
  someone moves it by hand. It checks the board only: the roadmap's MoSCoW
  list has no per-entry horizon, so neither "every issue is cited by an entry"
  (it flags refinements that were never features) nor "every unticked entry
  cites an issue" (it flags the design-first items the rule above *wants* to
  live in the roadmap alone) is checkable. Reconciling the roadmap stays a
  human read; the script's own docstring records why. Run it before a planning
  pass.
- **Filing an issue is not the end of filing it.** Since 2026-08-02 the
  project's *Auto-add to project* workflow puts every new issue on the board by
  itself, so an issue filed from now on is never *absent* — only unclassified,
  its `Horizon`, `Effort` and `Severity`/`Class` empty, which keeps it out of
  every prioritised view until someone fills them. Fill them in the same pass
  that files the issue. What escapes is not filing, it is filing *while doing
  something else*: #244, #246 and #248 were opened within seven hours of each
  other, each in the middle of the work that produced it, and none of the three
  reached the board at all until a check three days later put them there by
  hand. The auto-add is what turns that omission from invisible into
  reportable — an unclassified item is a `board-check` warning, where an
  absent one showed up nowhere but in the one command nobody had run.
- **`just roadmap` recompiles and validates the roadmap** — schema, duplicate
  ids, and links to a feature id nothing declares. It is the roadmap's
  counterpart to `board-check`: that one checks the board against the issues,
  this one checks the roadmap against itself. The `roadmap` CI job runs
  `validate` and then rebuilds and diffs, so a hand-edited `ROADMAP.md` or a
  forgotten regeneration fails the PR.
