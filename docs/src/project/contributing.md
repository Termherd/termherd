# Contributing

The full conventions live in the repository, and they are the authority:

- [`CONTRIBUTING.md`](https://github.com/Termherd/termherd/blob/main/CONTRIBUTING.md)
  — contribution conventions
- [`AGENTS.md`](https://github.com/Termherd/termherd/blob/main/AGENTS.md) — the
  engineering rules: dependency rule, quality bar, how work is tracked
- [`CODING_STANDARDS.md`](https://github.com/Termherd/termherd/blob/main/CODING_STANDARDS.md)
  — Tidy First, CUPID, YAGNI, TDD
- [`docs/CI.md`](https://github.com/Termherd/termherd/blob/main/docs/CI.md) —
  every gate, why it exists, and how to mirror it

## Before you push

Every gate below is blocking in CI. Mirror them locally:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check          # if cargo-deny is installed
cargo machete             # unused deps; if cargo-machete is installed
just check-deps           # the hexagonal crate dependency rule
just check-arch           # intra-crate module boundaries + OS-cfg containment
markdownlint-cli2         # markdown is gated too, including this book
```

CI runs each gate **only when its file category changed**, and they fan into
one required check. A docs-only pull request skips every Rust job.

## Building this book

```bash
just docs           # build → docs/book/index.html
just docs-serve     # live reload on http://localhost:3000
```

`create-missing = false` in `book.toml` means a link to a page that does not
exist **fails the build** rather than silently minting an empty page. The
chapter map in `SUMMARY.md` is a promise; that setting keeps it honest.

## Three rules that surprise newcomers

**No issue numbers in code.** Not in comments, doc-comments, or test names —
git already links code to its issue, and an in-code `#42` rots when issues are
renumbered. Cite issues in commit messages, PR bodies, and roadmap prose
instead.

**`ROADMAP.md` is generated — never edit it.** Edit the feature file under
`.roadmap/features/`, run `just roadmap`, and commit both. CI rebuilds and
diffs.

**A user-visible change updates this book in the same PR.** If what a user
sees, types or configures moves, the page describing it moves with it — not in
a follow-up. No gate catches a stale page: the `book` job proves the book
builds and that `SUMMARY.md` resolves, never that it still describes the
binary. Pages that restate what the code holds as data — the shortcut table,
the settings reference, the MCP tool tables — are the ones that rot first.
`AGENTS.md` carries the mapping from what you changed to what to update.

Everything written for the project is in English — commits, issues, pull
requests, comments, docs. The history is bilingual because the rule arrived
late; new writing is English whatever language the work was discussed in.

## How work is tracked

Three layers, each owning exactly one thing:

| Layer | Owns |
| --- | --- |
| `.roadmap/` + `docs/PRD.md` | the *what* and *why* — features, MoSCoW bucket, shipped history |
| GitHub issues | the *unit of work* — scoped, actionable, typed and labelled |
| The project board | *priority and order* — Horizon, Class, Effort, Severity |

The published view of the first layer is
[`ROADMAP.md`](https://github.com/Termherd/termherd/blob/main/ROADMAP.md):
every feature, its MoSCoW bucket, whether it has shipped, and the reasoning
behind it. It is compiled from `.roadmap/` — read it, never edit it. The board
that orders the work is internal, so `ROADMAP.md` is the answer to "does this
feature exist?" and the issue tracker to "is anyone on it?".

An epic graduates from the roadmap to an issue only when it is scoped enough to
act on.
