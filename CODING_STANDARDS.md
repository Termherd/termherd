# Coding Standards

This file is the working agreement for code in this repo. It is meant to be
re-read on a slow day, not skimmed once. Four pillars, in the order you
usually apply them:

1. **Tidy First** — separate behaviour changes from clean-ups.
2. **CUPID & YAGNI** — properties to aim for in design and refactoring.
3. **TDD (Red → Green → Refactor → Reflect)** — the loop that keeps the above
   honest.
4. **Clean Code** — local taste rules that survive automation.

Repo-specific rules take precedence when they collide. The authoritative
sources, in order, are [`AGENTS.md`](AGENTS.md), [`docs/PRD.md`](docs/PRD.md),
and [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). This file expands on the
*how*; those define the *what* and *why*.

---

## 1. Tidy First (Kent Beck)

> *Make the change easy, then make the easy change.*

Behaviour changes and structural changes are **two different commits**.

- **Tidying** — renames, extractions, dead-code removal, reformatting,
  splitting a long function, adding a missing test that pins existing
  behaviour. Never alters observable output.
- **Behaviour change** — the actual feature, fix, or contract change.

Rules of thumb:

- If the diff to add a feature feels too big, stop. Tidy the surrounding code
  first (in its own commit), then come back. The feature commit shrinks.
- Tests that pin existing behaviour are **Must-have**, not Could-have. Land
  them *before* the behaviour change. The point of pinning is to make the
  behaviour change reviewable as a small, intentional diff.
- If a tidy ends up changing observable behaviour, it wasn't a tidy. Revert
  and split.

Acceptable commit shapes:

```text
✅  refactor(core): extract allocate_session helper      (tidy)
    feat(m2): bound the PTY output channel                (behaviour)

❌  feat(m2): bound the channel + tidy session registry
```

---

## 2. CUPID & YAGNI

Five properties to optimise for, in roughly this order:

| Property            | One-liner                                               | Smell when violated                               |
| ------------------- | ------------------------------------------------------- | ------------------------------------------------- |
| **Composable**      | Plays well with others; small surface, no surprises.    | "I have to mock half the world to test this."     |
| **Unix philosophy** | Does one thing well.                                    | Module/trait with `and` in its purpose statement. |
| **Predictable**     | Behaves as expected; no hidden state, no spooky action. | "Works on my machine" / order-dependent tests.    |
| **Idiomatic**       | Reads like the language and the codebase.               | Reviewer says "this is clever" with a sigh.       |
| **Domain-based**    | Names match the product's vocabulary.                   | Generic `Manager`/`Helper`/`Util` names.          |

### C — Composable

Pieces plug together without special knowledge of each other.

- `core` is decoupled from `app`/iced; the headless `App` can be driven from a
  test harness with no GUI.
- Adapters plug in through port traits (`core::ports::{ProjectScanner,
  PtyHost}`); add, remove, or swap an adapter without touching `core`.
- `Event`/`Effect` are the contract between the shell and the core: `apply`
  consumes an `Event` and returns `Effect`s. The shell never reaches into core
  internals.

**Watch for**: shell code that re-implements domain decisions `core` should
own (focus routing, selection, status folding). If several call sites want the
same derived view, hoist it into `core` once instead of recomputing per site.

### U — Unix philosophy

Do one thing well.

- One tokio runtime, **actor-per-session**: each session task owns its PTY
  handle and terminal grid; everyone else talks to it over channels.
- One crate, one job: `claude` parses, `scan` walks the filesystem, `pty`
  drives terminals, `core` decides, `app` renders.

**Watch for**: an adapter that "while we're here" makes a domain decision. The
decision belongs in `core`; the adapter only performs the `Effect`.

### P — Predictable

Same input, same output, on any machine.

- `core::App::apply` is **pure**: no I/O, no clock, no panic. The same `Event`
  sequence yields the same `Effect`s anywhere.
- Deterministic iteration: if order matters, sort explicitly — don't rely on
  `HashMap`/`HashSet` iteration order.
- Broken invariants surface as `None`/`Err` (see the `Workspace` mutators),
  never via `unwrap` or a panic.

Non-deterministic work (filesystem, PTY, time, network) lives in adapters
behind ports, never in `core` or `claude`.

### I — Idiomatic

Feels like modern Rust to a Rust reader.

- Newtypes and enums to make impossible states impossible: `SessionId` over a
  bare `u64`, `Event`/`Effect`/`SessionStatus` as enums, not strings or bools.
- Iterator chains (`map`, `filter`, `fold`) over accumulating in a mut `Vec` —
  when it reads cleaner. Use a `for` loop when it doesn't.
- `Result<T, E>` and `Option<T>` over sentinel values. Pattern matching over
  nested `if let`.
- `thiserror` for library errors in `core`/`claude`; `anyhow` is acceptable at
  the `app` binary boundary for terminal errors. No `unwrap`/`expect`/`panic`
  in `core`/`claude` (clippy-denied) unless provably unreachable and commented.

For example, prefer a newtype over a bare integer so illegal states can't be
constructed:

```rust
// Bad: any u64 is a session id, including 0 or a stale one.
fn write(&self, session: u64, bytes: &[u8]) -> Result<(), PtyError>;

// Good: a session id is non-zero and minted only by the registry.
pub struct SessionId(pub NonZeroU64);
fn write(&self, session: SessionId, bytes: &[u8]) -> Result<(), PtyError>;
```

### D — Domain-based

The code speaks the product's vocabulary: sessions, panes, terminals.

- Types map to the domain: `Session`, `Workspace`, `Pane`, `Tab`, `Screen`,
  `Project`, `Event`, `Effect`, `SessionStatus`.
- File and module names name the concept, not the data structure:
  `workspace.rs`, `browser.rs`, `digest.rs`, `osc.rs`.
- Avoid `Manager`/`Helper`/`Util`. The Electron god-object is the cautionary
  tale this rewrite exists to escape (PRD §4).

**Watch for**: implementation-named helpers leaking into a port's public API.
Rename toward the domain when promoting something to the trait surface.

### YAGNI (anti-speculation rule)

CUPID describes what good code *is*; YAGNI protects against building what you
don't need yet.

- No abstraction for a second adapter that doesn't exist. A concrete adapter is
  fine until the next one actually lands (`mcp` is Unsure — don't pre-abstract
  for it).
- No trait with a single implementation — **except a port** that deliberately
  decouples `core` from an adapter. That is the hexagonal contract, not
  speculation; the distinction is whether the trait exists to invert a
  dependency or merely to look extensible.
- No field that duplicates information derivable from another (don't store what
  you can compute from the pane tree; derive it with a helper).
- No macro to remove scaffolding when the scaffolding *is* the contract.

When a refactor toward CUPID would require speculative work, stop and wait for
the second use case.

---

## 3. TDD with a fourth step — Reflect

The standard Red → Green → Refactor loop, with a deliberate **Reflect** beat at
the end of each cycle. The reflect step is what keeps the loop from grinding
out lots of small green tests that don't add up to a coherent design.

```text
   ┌──────────┐
   │   RED    │   Write the smallest failing test that names the
   │          │   behaviour you want. Run it. Confirm it fails for
   │          │   the right reason (not a typo, not an import).
   └────┬─────┘
        │
        ▼
   ┌──────────┐
   │  GREEN   │   Write the least code that makes the test pass.
   │          │   Ugly is fine here. Don't generalise yet.
   └────┬─────┘
        │
        ▼
   ┌──────────┐
   │ REFACTOR │   With the test green, clean up — names, duplication,
   │          │   shape. Tests stay green between every keystroke.
   │          │   This is a TIDY (see §1); commit it separately.
   └────┬─────┘
        │
        ▼
   ┌──────────┐
   │ REFLECT  │   Pause. Ask:
   │          │     • What did this cycle teach me?
   │          │     • What surprised me (red took longer? green was
   │          │       trivial? refactor revealed a missing concept)?
   │          │     • Is the *next* test on my list still the right
   │          │       one, or did this cycle change the plan?
   │          │     • Is there a test I should retire because it now
   │          │       overlaps with a stronger one?
   │          │     • Did I learn a domain rule worth pinning in
   │          │       another test, separate from the one I just
   │          │       wrote?
   │          │   Update the test list. Then loop.
   └────┬─────┘
        │
        ▼
       (next test)
```

Reflect rules:

- **Reflect is short.** A minute, sometimes thirty seconds. If it becomes a
  meeting, do it asynchronously between cycles.
- **Reflect updates the plan, not the code.** If reflection reveals code that
  should change, that's the *next* RED test, not an edit you smuggle into the
  current cycle.
- **Reflect after Green-but-no-Refactor cycles too.** "There was nothing to
  clean" is itself a signal — either the design is good or you're not looking
  hard enough.
- **Always surface findings to the user with a recommendation.** Every
  reflection that produces a finding (a new test worth pinning, a surprise that
  suggests a missing concept, a smell you noticed) gets a one-line decision
  prompt: *"apply now / add to today / add to the roadmap / forget it"*. Do not
  silently carry findings forward and do not silently apply them.

  Recommend the best move per the principles, and say *why* in one short
  clause. Heuristics:
  - **Apply now** — the finding closes a still-open hole from the cycle just
    finished, the fix is small, and skipping it would leave the work
    half-done. (e.g. forward contract test landed → reverse test is ~30 LOC →
    apply now closes the lesson.)
  - **Add to today** — same-session work, but it would derail the current
    task; better as the next discrete cycle.
  - **Add to the roadmap** — useful but not on the critical path; capture it in
    [`ROADMAP.md`](ROADMAP.md) so it's not lost.
  - **Forget it** — speculative, low-leverage, or you're not sure it's real.
    Recording every passing thought is its own debt.

  Default leans toward *apply now* when the finding is small and directly tied
  to the cycle that surfaced it (Tidy First: keep the diff coherent). Lean
  toward *roadmap* when the finding is larger than the cycle it interrupted
  (CUPID-Composable: don't bundle unrelated work).

### Testing in `termherd`

- **Unit tests** live in the same file, under `#[cfg(test)] mod tests`. `core`
  and `claude` are pure, so they are exhaustively unit-tested — there is no I/O
  excuse. A `Workspace` mutator (`open`/`split`/`focus`) gets at least a
  positive, a negative, and an edge case, and an invariant violation **must**
  return `None`/`Err` with a test that pins it.
- **Property tests** use `proptest` (already a dev-dependency in `claude`) for
  codec round-trips and invariants — e.g. every derived `cwd` re-encodes to its
  folder name.
- **Integration tests** for adapters live in `tests/`. The codec is validated
  against a real `~/.claude/projects` tree (every session digests, every cwd
  re-encodes).
- **Tests may use `unwrap`/`expect`/`panic`** — `clippy.toml` allows it in
  tests so the discipline doesn't fight the harness. Production paths may not.
- CI runs the suite via `cargo nextest run --workspace`; locally `cargo test
  --workspace` is fine.

---

## 4. Clean Code

Local taste rules. None of these are absolute; they exist to be broken *on
purpose*, not by accident.

### Names

- A name should let a reader skip the implementation. If they have to read the
  body to understand the name, rename it.
- Domain words beat generic ones.
- Boolean names read as predicates: `is_live`, `has_focus`, `should_carry`. Not
  `flag`, not `status` (unless it's an enum like `SessionStatus`).
- Types: `PascalCase`; functions/vars: `snake_case`; constants:
  `SCREAMING_SNAKE_CASE`; enum variants (`Event::LaunchSession`): `PascalCase`;
  files: `snake_case.rs`.

### Functions

- One purpose per function. If you'd need "and" to describe it, split.
- Short by default — long when the alternative is a tangle of helpers no one
  will read in order.
- Arguments: 0–3 is fine; 4+ wants a struct or builder (see `SpawnSpec` for the
  PTY spawn call instead of five positional args).
- No flag arguments that change *what* the function does. `do(x, dry_run:
  bool)` is fine (toggles a side-effect); `do(x, mode: Mode)` is usually two
  functions.

### Comments & Documentation

- Default to **no inline comments**. Code says *what*; commit messages and PR
  descriptions say *why*.
- Write an inline comment only when the *why* is non-obvious from the code: a
  hidden constraint, a surprising invariant, a workaround for a specific bug
  (e.g. the `ESC[6n` responder that ConPTY needs before it starts the child).
- Every public item has a doc comment: a one-line summary, then details and
  examples, using intra-doc links.

```rust
/// Drives one terminal session: spawn, write, resize, kill.
///
/// Implemented by the [`pty`] adapter as an actor-per-session; `core` performs
/// terminal [`Effect`]s against this port and never touches a PTY directly.
pub trait PtyHost {
    // ...
}
```

### Errors

- Validate at boundaries (CLI args, file I/O, PTY spawn). Trust internal
  callers.
- **Fail loudly and early.** A silent catch is not a style preference here — it
  is one of the four quality gaps this rewrite exists to fix (PRD §4). A
  swallowed error is a future bug report; surface it as `Err` or `tracing`,
  never an ignored `Result`.
- `core` and `claude` return typed `thiserror` errors. The `app` binary handles
  terminal errors at the boundary (`anyhow` acceptable there).
- **One logging stack: `tracing`.** No `println!` outside tests.

---

## Commit style

Commits follow [Conventional Commits](https://www.conventionalcommits.org/):

```text
<type>(<optional scope>): <short summary>

<optional body>

<optional footer>
```

Types:

- `feat`: new feature or capability
- `fix`: bug fix
- `docs`: documentation only
- `refactor`: code change that neither fixes a bug nor adds a feature
- `test`: adding or fixing tests
- `chore`: build tooling, dependencies
- `perf`: performance improvement

Scopes in this repo are either a **milestone** (`m0`, `m1`, `m2`) or a
**crate/area** (`core`, `claude`, `pty`, `scan`, `app`, `ci`, `dist`, `docs`).

Breaking changes include `BREAKING CHANGE:` in the footer or `!` after the
type. Commit messages carry **no "Claude" signature** (per the global user
instruction).

---

## Toolchain & Layout

### Workspace layout

Hexagonal workspace — adapters depend on `core`, never the reverse:

```text
crates/
├── core/    # domain: App state machine, Workspace, ports, keymap (no I/O)
├── claude/  # pure CLI-format codec: path · derive · digest · osc · jsonl
├── scan/    # filesystem discovery adapter (implements ProjectScanner)
├── pty/     # terminal adapter, actor-per-session (implements PtyHost)
└── app/     # iced GUI shell; constructs adapters in main() and injects them
```

When adding code, ask *which crate does this belong in?* If the answer is
"`core` should call this adapter directly," the answer is wrong — add a port
trait in `core::ports` and have the adapter implement it.

### Toolchain policy

- `rust-toolchain.toml` pins **exactly `1.95.0`, edition 2024** (Q10). Unlike
  projects that track `stable`, termherd pins an exact minor version — do **not**
  bump it without updating the pin and re-checking CI in the same commit.
- `Cargo.toml [workspace.package]` declares `rust-version = "1.95"`. Keep it in
  lockstep with the toolchain pin; bumping one without the other is a bug.

### Clippy & lints

Lints are declared once at the workspace root and tightened per crate:

- `[workspace.lints.rust]`: `unsafe_code = "deny"` everywhere.
- `[workspace.lints.clippy]`: `unwrap_used`, `expect_used`, `panic`, `todo`,
  `unimplemented` = `warn` by default.
- `core` and `claude` tighten `unwrap_used`/`expect_used`/`panic` to **`deny`**
  in their own `Cargo.toml` (Q5): production code in those crates must return
  typed errors.
- `clippy.toml` opts tests out (`allow-unwrap-in-tests` and friends).
- CI runs `cargo clippy --workspace --all-targets -- -D warnings`: **every
  warning is blocking.** Mirror it locally before pushing.

### CI gates (all blocking)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace          # `cargo test --workspace` locally
cargo deny check                       # if cargo-deny installed
cargo machete                          # unused deps; if cargo-machete installed
just check-deps                        # hexagonal crate dependency rule
markdownlint-cli2                      # 80-col prose; see .markdownlint-cli2.jsonc
```

CI also runs `actionlint` (workflow YAML) and CodeQL (SAST) — those aren't part
of the routine local loop. For the **full picture** — every gate, what it
protects, when it runs (PR / merge / schedule / release), how to mirror it, and
the sanctioned exceptions — see the CI reference & runbook at
[`docs/CI.md`](docs/CI.md).

Markdown prose wraps at **80 columns** (tables and code blocks are exempt). This
file obeys that rule; keep it that way when you edit.

---

## Review mindset

When reviewing a change, ask:

1. Does it meet the design principles above?
2. Does it respect the dependency rule, and does domain logic live in `core`
   (not the shell)?
3. Are there impossible states the types now allow?
4. Is any error silently swallowed? (The cardinal sin — PRD §4.)
5. Is the logic tested, and is the documentation up to date?
6. Would a future maintainer understand *why*, not just *what*?

Kindness over pedantry. The goal is a better codebase, not a perfect one.
