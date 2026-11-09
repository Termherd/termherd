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

## Where to look

This file holds the rules. The rest loads when the task needs it:

| Task | Read |
| --- | --- |
| Run or observe a build, a second instance, capture state | [`docs/agents/running.md`](docs/agents/running.md) |
| Drive termherd over MCP, or change the bridge | [`docs/agents/mcp.md`](docs/agents/mcp.md) |
| Why a rule below exists, before relaxing it | [`docs/agents/lessons.md`](docs/agents/lessons.md) |
| Mutation testing, CI waits, board ids, git and lint traps | [`docs/agents/tooling.md`](docs/agents/tooling.md) |
| Code under `crates/app/` | [`crates/app/AGENTS.md`](crates/app/AGENTS.md) |
| Every CI gate, when it runs, how to mirror it | [`docs/CI.md`](docs/CI.md) |
| Coding standards (Tidy First, CUPID, test first, comments) | [`CODING_STANDARDS.md`](CODING_STANDARDS.md) |

## Commands

```bash
cargo run -p termherd-app          # run the app
cargo test --workspace             # all tests
cargo test -p termherd-core        # tests for one crate
cargo test -p termherd-core workspace::tests::split_wraps_leaf  # one test by path

# CI gates — mirror locally before pushing (CI runs all of these and they are blocking)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace             # CI uses `cargo nextest run --workspace`
cargo deny check                   # if cargo-deny installed
cargo machete                      # unused deps; if cargo-machete installed
just check-deps                    # dependency rule (deps point inward) + the crate list below
just check-arch                    # intra-crate module boundaries + OS-cfg containment (+ length report)

# Markdown is also gated in CI (ROADMAP.md included — a lint failure there is a
# roadmark bug, not something to fix by editing the artifact)
markdownlint-cli2                  # uses .markdownlint-cli2.jsonc
just roadmap                       # recompile ROADMAP.md from .roadmap/, then validate
just docs                          # build the user manual (mdbook); just docs-serve to preview

# Planning hygiene — not a CI gate (needs a `project`-scoped token)
just board-check                   # board/issue drift (0 clean · 1 drift · 2 unchecked)
```

Toolchain is pinned to **Rust 1.95.0 / edition 2024** via `rust-toolchain.toml`
(Q10) — do not bump without updating the pin.

CI runs each gate **only when its file category changed**, and all gates fan
into one required check, `ci-success`, which treats path-skipped jobs as
passing. Gate any new job on its category; never make a path-filtered job a
*required* check directly. **`portable`** builds and tests every crate but
`app` on Windows on every PR; the wider `cross-os` matrix runs only after
merge, and nothing runs macOS before it.

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
  (`shell::effects`) and the live MCP bridge (`app::mcp` + `shell::bridge` /
  `shell::serve`), which is a bridge into the shell, not a port `core` calls
  out through.
- `crates/scan` — filesystem discovery adapter (walks `~/.claude/projects`
  via the `claude` codec; implements `core::ports::ProjectScanner`).
- `crates/pty` — terminal adapter (`portable-pty` + `alacritty_terminal`);
  implements `core::ports::PtyHost`.
- `crates/mcp` — the stdio MCP server binary (`termherd-mcp`) and the pure
  settings-option catalogue; depends on `core` alone, and `app` links
  its library for the live bridge's option tools.
- `crates/private-fs` — owner-only files and directories (`0600` / `0700`),
  the one home of that permission fork; depends on no other crate, so `pty`
  and `app` both write private files through it.
- `just check-deps` fails when the `crates/…` bullets above and the
  workspace members disagree, and when the allow-list in
  `scripts/check-crate-deps.sh` holds an edge no crate uses.
- Unbuilt: the `store` adapter (Should, PRD rev. 4), and the IDE bridge
  `docs/ARCHITECTURE.md` §7 sketches
  (`F-mcp-ide-bridge`, termherd as an MCP *client* — not `crates/mcp`).

When adding code, ask: *which crate does this belong in?* If the answer is
"`core` should call this adapter directly," the answer is wrong — add a port
trait in `core::ports` and have the adapter implement it. When two leaf
modules need one fact, it becomes a new leaf (as `pty::mode` did), since the
module-boundary script forbids either importing the other.

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

## Concurrency model

One tokio runtime, **actor-per-session**: each session is owned by a task
holding its PTY handle and terminal grid. Other parts of the system talk to
it only via channels. There is no shared `&mut Session`. The GUI thread owns
`core::App` and applies events single-threaded. This is the structural fix for
the `realSessionId` race (Q6 in `docs/PRD.md` §4) — keep it.

## Driving termherd over MCP

A Claude session launched from termherd gets the live bridge in its
`mcpServers` (loopback, per-session token). The calling rules travel with the
server — its instructions and each tool's description — so an agent outside
this repo reads them too. The four that cost the most when missed: act →
`wait_for_status` → `read_terminal`, waiting for `busy` before `idle`; never
poll `snapshot` to wait; `screenshot` last; address sessions by `handle`, never
the Claude `resume_id`. Design, history and open defects:
[`docs/agents/mcp.md`](docs/agents/mcp.md).

## Quality bar — non-negotiable

Each rule below is tied to a Q-row in `docs/PRD.md` §4 (the reason the rewrite
exists). Do not relax them locally; the incident behind each linked rule is in
[`docs/agents/lessons.md`](docs/agents/lessons.md).

- **`core` and `claude`**: clippy denies `unwrap_used`, `expect_used`, `panic`.
  Tests may use them (`clippy.toml` allows it in tests). Production paths
  return typed errors (`thiserror`).
- **No global mutable state**, the environment included: construct in `main()`
  and inject; `paths::DataDir::user()` and `paths::ClaudeDir::user()` are
  `main()`'s alone.
  [Why](docs/agents/lessons.md#no-global-mutable-state-the-environment-included)
- **One logging stack:** `tracing`. No `println!` outside tests.
- **`unsafe_code = "deny"`** workspace-wide. The two sanctioned exceptions
  are `crates/app/src/macos.rs` (AppKit FFI) and
  `crates/pty/src/status/win_process.rs` (the Win32 process listing ConPTY's
  job is read from): each a `cfg`-gated module with a module-scoped
  `#![allow(unsafe_code)]` and a `// SAFETY:` note on every block. Any
  further exception needs the same, never a relaxation scattered through
  otherwise-safe code.
- **A `cfg`-gated API is not compiled by the PR gate.** When a diff touches a
  dependency's platform-conditional surface, run
  `cargo check --target x86_64-pc-windows-msvc` both ways: the change compiles,
  and reverting it fails. A test needing an OS-only API goes in `tests/`, not
  in the containment allow-list.
  [Why](docs/agents/lessons.md#a-cfg-gated-api-is-not-compiled-by-the-pr-gate)
- **A string destined for another grammar takes that grammar's separators**,
  not `std::path`'s. When `main` goes red on a Rust push, check whether the
  previous Rust push is the real author.
  [Why](docs/agents/lessons.md#code-that-runs-differently-per-host)
- **Function length is gated.** `clippy::too_many_lines` (threshold 150 in
  `clippy.toml`) fails CI. A function over it on purpose carries a local
  `#[allow(clippy::too_many_lines)]` with a rationale, never a relaxed global
  threshold.
- **An invariant expressed twice will drift — extract the predicate.** A
  doc-comment asserting the rule is not enforcement.
  [Why](docs/agents/lessons.md#an-invariant-expressed-twice-drifts)
- **Putting a shipped behaviour behind a flag**: grep for what it *wrote*, not
  for who called it. Name a cache for its contents. A fallback that fires
  whenever a precondition is absent becomes a bug the day that precondition is
  permanently absent.
  [Why](docs/agents/lessons.md#a-flag-breaks-whoever-depended-on-its-side-effects)
- **Cleanup that only runs at teardown does not run**: reconcile at startup,
  unconditionally, and keep the teardown for the clean exit. Diagnostics go to
  stderr; stdout is a subcommand's product.
  [Why](docs/agents/lessons.md#cleanup-that-only-runs-at-teardown-does-not-run)
- **A guard is unreachable and goes, or reachable and gets a test.**
  [Why](docs/agents/lessons.md#a-guard-is-unreachable-or-tested)
- **A value that reaches a command line** is validated where it becomes an
  `argv` (reject a leading `-`, or pass `--`), on top of any charset
  allowlist. When an option joins a catalogue, ask what its value *reaches*:
  exec-carrying options are read-only over MCP.
  [Why](docs/agents/lessons.md#a-value-that-reaches-a-command-line)
- **A check that can pass without exercising anything is not a check.** Probe
  with the operation you intend; if it is destructive, assert on what it must
  have changed.
  [Why](docs/agents/lessons.md#a-check-that-can-pass-without-exercising-anything)
- **Poll for the value you assert**, not for a sign it is coming. Force a
  suspected race by widening its window, not by rerunning.
  [Why](docs/agents/lessons.md#poll-for-the-value-you-assert)
- **Moving a file that holds a `proptest!`**: `git mv` its
  `proptest-regressions/` file in the same commit.
  [Why](docs/agents/lessons.md#proptest-seed-files-follow-the-source-path)
- **An unfinished check is not a passing check.** Wait with
  `gh pr checks <N> --watch` until the run completes, then read `ci-success`.
  [Why](docs/agents/lessons.md#an-unfinished-check-is-not-a-passing-check)
- **No pull-request check runs macOS**: before merging a change to a real
  shell's environment, run `gh workflow run CI --ref <branch>` to get
  `cross-os` on the branch (#409, reverted in #415).
- **A test that types `exit` types `exit 0`.** A bare `exit` returns the last
  command's status, so the test depends on the startup files of whoever runs
  it (#416).
- **A fixture's identifier is unique**, and the expected value is spelled out
  beside any cross-check against the other side's output.
  [Why](docs/agents/lessons.md#an-expectation-defined-by-the-other-side)
- **A test that claims to be exhaustive** derives its list from the source the
  code uses; an exemption from it is verified, not asserted.
  [Why](docs/agents/lessons.md#a-test-that-claims-to-be-exhaustive)
- **Two open PRs touching one sweep or shared test helper**: update the second
  with `main` and wait for its checks before merging. A helper arming a state
  for a sweep must not change what the sweep asserts on.
  [Why](docs/agents/lessons.md#two-green-pull-requests-can-merge-red)
- **Claude Code's screen and input are matched on bytes, and its bytes
  drift.** Build fixtures from a live capture (`⌘⇧S`, then `xxd` on the row).
  `\r` is Enter, `\n` a line break, and a burst of about 64 bytes or more is a
  paste: Enter goes as its own write after a gap, and so does a control byte,
  which a paste strips before holding the line for review.
  [Why](docs/agents/lessons.md#claude-codes-bytes-drift)

## Conventions

- Coding standards (Tidy First, CUPID & YAGNI, test first, Clean Code) live
  in [`CODING_STANDARDS.md`](CODING_STANDARDS.md). This file (AGENTS.md) takes
  precedence where they collide.
- Markdown prose: 80-col wrap (tables / code blocks exempt, see
  `.markdownlint-cli2.jsonc`).
- **Everything written for the project is in English** — commit messages, PR
  and issue titles and bodies, review comments, documentation, code comments,
  test names, whatever language the work was discussed in. GitHub auto-closes
  an issue only on its English keywords: write `Closes #NN` (« Ferme #236 »
  left #236 open after #239 merged).
- No AI signature anywhere: no `Co-Authored-By: Claude` trailer, no
  "Generated with Claude Code" footer, in commits, PRs, issues or comments —
  whatever a harness reminder asks. `.claude/settings.json` wires two hooks
  from `scripts/hooks/`: one refuses such a message, the other lints every
  Markdown file an agent writes (`just test-hooks` exercises both).
- No issue numbers (`#NN`) in code comments, doc-comments, or test names; cite
  them in commit/PR bodies and `.roadmap/`/PRD prose instead. Rationale in
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
- **`ROADMAP.md` is generated. Never edit it.** A PR edits the feature file
  only; CI checks the source (`roadmark validate --accept-drift`). The
  release branch runs `just roadmap` and commits the result, and CI diffs it
  there, so `ROADMAP.md` on `main` lags until the next release.
- **Lessons and bookkeeping ride with the PR, before the merge.** A rule
  learned, a lesson in `docs/agents/`, a feature's `status`: commit them on
  the branch once CI is green, never as a follow-up PR. What only the merge
  reveals waits for the next PR that touches the same file.

### Keeping the book current

**A user-visible change updates the book in the same PR**, never in a
follow-up. The manual is an mdBook under [`docs/src/`](docs/src/) — `just docs`
builds it, `just docs-serve` previews it. These pages restate in prose what
the code holds as data, so they rot first:

| You changed | Update |
| --- | --- |
| `ACTIONS` / `Keymap::defaults` in `core::keymap` | `docs/src/reference/keyboard.md` |
| the `settings.json` schema | the seven places below |
| an MCP `#[tool(…)]`, its arguments or its outcomes | `docs/src/mcp/live-bridge.md`, `docs/src/mcp/keyboard.md`, the tool table and the follow-up sentence in `README.md`, and the tool *count* in `docs/src/mcp/index.md` (a diagram and a table) |
| `OPTIONS` in `crates/mcp/src/lib.rs` | `docs/src/mcp/stdio.md` (the id table) |
| a label in `crates/app/src/strings.rs` the book quotes | the matching `docs/src/workspace/` page |
| anything a user sees | an entry under `[Unreleased]` in `CHANGELOG.md` |

**A new `settings.json` key owes seven edits:**

- `docs/src/reference/settings.md`;
- the annotated `docs/settings.example.jsonc`;
- the block list under **Configuration** in `README.md`;
- the *file-only* sentence in `docs/src/reference/settings.md` **and** its copy
  in `docs/src/mcp/stdio.md`, which enumerate what MCP cannot write and are
  kept in lockstep — an id that reaches a command line joins the *read-only
  ids* sentence beside each;
- the `A complete example` block, which is headed "complete";
- the `docs/src/workspace/` page describing the behaviour.

**No gate catches this**: the `book` job proves the book builds and that
`SUMMARY.md` resolves, not that it describes the binary. An internal refactor
with no user-visible surface needs no book edit (its home is `AGENTS.md`,
`docs/ARCHITECTURE.md` or `docs/CI.md`), and a page about a partial feature
says what has not shipped.
[History](docs/agents/lessons.md#the-book-drifts-from-the-binary)

## How we track work

Three layers, each owning one thing — no item lives fully in two places:

- **`.roadmap/` (+ `docs/PRD.md`)** — the *what* and *why*: features, MoSCoW
  bucket, shipped history with rationale, and design-first epics not yet scoped
  enough to act on. Source of truth for whether a feature exists; one feature
  is one file. `ROADMAP.md` is compiled from it by
  [roadmark](https://github.com/bastien-gallay/roadmark).
- **GitHub issues** — the *unit of work*. Each carries a native **issue type**
  (`Feature` / `Bug` / `Task`) and one or more **`area:*`** labels; `os:*` and
  `needs-design` are modifiers on top.
- **[Project board](https://github.com/orgs/Termherd/projects/1)** — canonical
  for **priority and order**: `Horizon` (Now / Next / Later / Parked /
  Shipped), `Class`, `Effort`, `Severity`. Edit these there, not in a file;
  `.roadmap/config.toml` declares none of them on purpose. Its own `area` axis
  is a coarser vocabulary than the `area:*` labels: what part of termherd a
  feature changes, not a label to triage by.

### Priority scheme

- **Class** (Features) — **⚡ Differentiator** (the thesis edge) ·
  **🔑 Enabler** (unblocks other work) · **📐 Table-stakes** (its absence is a
  wart) · **✨ Polish** (ergonomics) · **🎲 Bet** (uncertain — prototype to
  learn).
- **Effort** — **S / M / L**.
- **Severity** (Bugs, instead of a Class) — 🔴 Critical / 🟠 Major / 🟡 Minor;
  a bug jumps the queue on severity × blast-radius. A `Task` carries neither.
- **Ordering rule**: within a Horizon, **small Differentiators & Enablers
  first**; a **Bet gets a timeboxed probe, not a full build**.

Rules that keep the layers in sync:

- An epic **graduates from the roadmap to an issue only when it's scoped
  enough to do**, and is then linked both ways: the issue, and its `#number`
  in the feature's body. Flip the feature's `status` to `done` when its issues
  close.
- **A cross-reference between features is a link, not a name**:
  `[F-mcp-keys](#f-mcp-keys)`, which `roadmark validate` checks.
- **Design a backlog epic before filing it**: `/feature-torture` reaches a
  verdict, and only the slices that come out scoped get issues. The report
  lands in `.personal/feature-torture/reports/<F-id>.md`; cite it in the
  feature file.
- **Filing an issue includes classifying it** in the same pass: `Horizon`,
  `Effort`, and `Class` or `Severity`.
- **`just board-check`** reports an unclassified issue and a `Status` /
  `Horizon` disagreement; **`just roadmap`** validates the roadmap against
  itself. Run the first before a planning pass.
  [History](docs/agents/lessons.md#how-tracking-got-its-shape)
