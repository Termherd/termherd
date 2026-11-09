# Coding Standards

This file is the working agreement for code in this repo. It is meant to be
re-read on a slow day, not skimmed once. Four pillars, in the order you
usually apply them:

1. **Tidy First** — separate behaviour changes from clean-ups.
2. **CUPID & YAGNI** — properties to aim for in design and refactoring.
3. **Test first** — design, failing tests, implementation, then one
   simplify-and-review pass over the diff.
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
- New tests, and the scaffolding that lets a red test compile (a new enum
  variant, a no-op message), are behaviour work: the tidy commit lands before
  them, with the suite green.

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
  fine until the next one actually lands (the IDE bridge `F-mcp-ide-bridge` is
  Unsure — don't pre-abstract for it).
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

## 3. Test first, one refactor at the end

Work in this repo is done almost entirely by agents, and agents never kept the
per-cycle Red → Green → Refactor → Reflect loop. What works for them is
**test first**: design the change, write the failing tests, implement, then
clean up once over the whole diff.

> **Why not TDD.** Dropping the TDD loop is a pragmatic choice for agents, not
> a verdict on TDD. Agents did not keep its per-cycle discipline, and test first
> with one refactor at the end gave them better results. A human contributor
> should still prefer TDD (Red → Green → Refactor, with a short Reflect after
> each cycle): the small cycles are where it pays off for a person.

One issue runs through these steps:

```text
   ┌──────────────┐
   │ FRAME+DESIGN │   Read the issue, its board fields and the feature
   │              │   file. Decide the crate, any port trait, the
   │              │   cfg-gated surface, the error types, and the tests
   │              │   to write. Apply the AGENTS.md quality bar here,
   │              │   before any code. Branch from an up-to-date main.
   └──────┬───────┘
          ▼
   ┌──────────────┐
   │ TIDY         │   Only if the change is hard to make: prepare the
   │ (optional)   │   code in its own commit, suite green (see §1).
   └──────┬───────┘
          ▼
   ┌──────────────┐
   │ TESTS FIRST  │   Write the failing tests that name the behaviour.
   │              │   Run them. Confirm each fails for the right reason
   │              │   (not a typo, not an import). A bug's first test
   │              │   reproduces it.
   └──────┬───────┘
          ▼
   ┌──────────────┐
   │ IMPLEMENT    │   The code that turns them green, within the rules
   │              │   decided at design time.
   └──────┬───────┘
          ▼
   ┌──────────────┐
   │ SIMPLIFY     │   `/simplify` over the branch diff: reuse, dead
   │              │   weight, shape. The one refactor step. It leaves the
   │              │   work clean for whoever comes next; Tidy prepared
   │              │   it, the two are not the same step.
   └──────┬───────┘
          ▼
   ┌──────────────┐
   │ REVIEW       │   `/code-review` on the branch diff (`low` for an S,
   │              │   `medium` for an M), with the review mindset below
   │              │   and the AGENTS.md quality bar as its grid.
   └──────┬───────┘
          ▼
   ┌──────────────┐
   │ GATES + PR   │   Local gates, the book and CHANGELOG.md in the same
   │              │   PR, then wait for `ci-success` to finish. Lessons
   │              │   and the feature's `status` go on the branch now,
   │              │   before the merge: no follow-up PR.
   └──────┬───────┘
          ▼
   ┌──────────────┐
   │ CLOSE        │   After the merge: the board, and the worktree
   │              │   removed. Nothing left to commit.
   └──────────────┘
```

Simplify and review run once per branch, after the tests are green, so the
review reads the code that will merge rather than code simplify then changes.
On a one-file fix, the review alone is enough.

**Surface findings to the user as a decision.** A finding from any step (a
test worth pinning, a missing concept, a smell) gets a one-line decision
prompt: *"apply now / add to today / add to the roadmap / forget it"*. Do not
silently carry findings forward and do not silently apply them.

The user picks. Give a recommendation only when asked, then name the move and
its *why* in one short clause. Heuristics:

- **Apply now** — the finding closes a still-open hole in the work just
  finished, the fix is small, and skipping it would leave the work half-done.
- **Add to today** — same-session work, but it would derail the current task;
  better as the next discrete step.
- **Add to the roadmap** — useful but not on the critical path; capture it as a
  feature file in `.roadmap/features/` (never in the generated `ROADMAP.md`) so
  it's not lost.
- **Forget it** — speculative, low-leverage, or you're not sure it's real.
  Recording every passing thought is its own debt.

*Apply now* fits a finding that is small and directly tied to the work that
surfaced it (Tidy First: keep the diff coherent); *roadmap* fits one larger
than that work (CUPID-Composable: don't bundle unrelated work).

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

**Write short.** Comments, doc comments, every `.md` (book, `CHANGELOG.md`,
`AGENTS.md`), commit messages, pull requests, issues and review comments use
the fewest plain words that keep the fact.

A comment must pass these, in order:

1. **Refactor it away.** If extracting a function, variable or type lets the
   code say it, do that and drop the comment. A heading over a block is a
   function name; a comment per `match` arm is a set of named handlers.
2. **Last.** Keep only a doc comment, or what the code cannot state: a hidden
   constraint, a platform quirk, a workaround (the `ESC[6n` responder ConPTY
   needs before it starts the child).
3. **Unique.** No history, issue number, author or file name: git and the PR
   hold them.
4. **Short and factual.** A fragment is fine when it reads clearly.
5. **Why, not how or what.** The code shows how; names say what.

- These rules override the surrounding density: thin over-commented code as
  you pass through.
- A reference code resolves without context: name the rule, or use `FRn`
  (defined in [`AGENTS.md`](AGENTS.md) conventions).
- Every public item has a doc comment: one summary line, then only the
  constraints its signature cannot state.
- Test: the name states the intent; a comment is one line of non-obvious why.
- Changelog entry: one or two lines of what the user sees. Book page: what the
  user sees or does, not the mechanism.

Outside the code, rules 3 to 5 hold, except that issue numbers belong there
(`Closes #NN`):

- Commit body: the why, in a few lines; the diff shows the what.
- Pull request: why, the change in a few bullets, how it was verified. No
  restated diff, no commit list.
- Issue: observed, expected, steps to reproduce. A guess is labelled as one.
- Review comment: the defect and the fix, in one or two lines.

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
- `ci`: CI workflows
- `perf`: performance improvement

The scope names where the change lands: a crate (any workspace member, by
its directory name: `core`, `pty`, …), a module or surface (`tabs`,
`keymap`, `instance`), or a doc area (`agents`, `standards`, `roadmap`, `ci`).

Breaking changes include `BREAKING CHANGE:` in the footer or `!` after the
type. No AI signature, in commits or anywhere else — see the conventions in
[`AGENTS.md`](AGENTS.md).

---

## Toolchain & Layout

### Workspace layout, toolchain and CI

The crate map and the dependency rule are in [`AGENTS.md`](AGENTS.md)
(*Architecture*), the local commands under its *Commands*, and every CI gate
in [`docs/CI.md`](docs/CI.md). Two rules live only here:

- `Cargo.toml [workspace.package]` declares `rust-version = "1.95"`. Keep it
  in lockstep with the `rust-toolchain.toml` pin; bumping one without the
  other is a bug.
- Lints are declared once at the workspace root and tightened per crate:
  `unsafe_code = "deny"` everywhere; `unwrap_used`, `expect_used`, `panic`,
  `todo`, `unimplemented` warn by default, and `core` and `claude` raise the
  first three to `deny` (Q5). `clippy.toml` opts tests out, and CI runs clippy
  with `-D warnings`, so every warning is blocking.

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
