# Contributing to TermHerd

Thanks for helping build TermHerd. This file collects the contribution
conventions that don't live in code. The broader engineering rules — the
hexagonal dependency rule, the quality bar, how we track work — live in
[`AGENTS.md`](AGENTS.md) and [`CODING_STANDARDS.md`](CODING_STANDARDS.md);
read those before non-trivial work.

## Issue references belong in git, not in code

**Don't put issue numbers (`#NN`) in code comments, doc-comments, or test
names.** Link code to the issue it came from through git — the commit and PR
that introduced the change — not through a number baked into a source line.

Why:

- **Git never rots.** `git blame → commit → PR → issue` always resolves to the
  live discussion. An in-code `#42` is a dead string: it breaks silently when
  issues are renumbered, migrated between trackers, or closed, and a reader
  can't click it.
- **It's noise at the wrong altitude.** A comment should say *what the code
  does and why*, in terms that outlive any one ticket. The ticket number is
  provenance — and provenance is git's job.

Where issue numbers **do** belong:

- **Commit messages and PR descriptions** — cite `#NN`, `Closes #NN`, etc.
  freely. This is the durable link.
- **`ROADMAP.md` and `docs/PRD.md` prose** — feature epics reference their
  tracking issues so the roadmap and the board stay in sync.

Where they **don't**:

- Code comments (`//`, `///`, `//!`) and any string that reads as
  documentation (e.g. an `assert!` message).
- Test names. Name a test for the behaviour it pins (`an_idle_tab_quits_silently`),
  not the ticket that prompted it.

Requirement tags the codebase already uses — `FR5`, `G1`, feature slugs like
`F-capture` — are fine; they name stable design artifacts, not tickets.

**Applying it in practice:**

- *New code:* no `#NN` in comments or test names.
- *Boy-scout:* when you rewrite an existing comment for another reason, drop
  its `#NN` while you're there.
- *No retroactive crusade:* don't mass-strip `#NN` from code you aren't
  otherwise touching — it churns `git blame` for no functional gain.

## Before you push

Mirror the CI gates locally (they are all blocking):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
markdownlint-cli2                  # markdown is gated too
```

See [`docs/CI.md`](docs/CI.md) for the full gate reference and
[`AGENTS.md`](AGENTS.md) for everything else.
