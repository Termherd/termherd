# CI gate map

A visual companion to [`CI.md`](CI.md), which stays the **source of truth** for
every gate. This page is the fast mental model: what runs, gated by what, and
the single check that guards `main`. A full-colour version lives alongside it in
[`ci-gate-map.html`](ci-gate-map.html) (open in a browser; GitHub shows the
source, not the render).

## The PR merge gate

Every push runs only the checks its diff can affect. A `changes` classifier
(`dorny/paths-filter`) tags the diff, each gate fires on its category, and they
all fan into one required check.

```text
Pull request → main
═══════════════════

        ┌─────────────────────────────────────────────────┐
        │  changes · dorny/paths-filter                    │
        │  classifies the diff into four booleans:         │
        │  rust · cargo · markdown · workflows             │
        └────┬──────────┬──────────────┬──────────┬────────┘
             │ rust     │ cargo        │ markdown │ workflows
             ▼          ▼              ▼          ▼
        ┌─────────┐┌───────────────┐┌───────────┐┌────────────┐
        │ rustfmt ││ cargo-deny    ││ markdown- ││ actionlint │
        │ clippy  ││ cargo-machete ││   lint    ││            │
        │ test    ││ dependency-   │└───────────┘└────────────┘
        └─────────┘│    rule       │
                   └───────────────┘
             │          │              │          │
             └──────────┴───────┬──────┴──────────┘
                                ▼
        ╔═════════════════════════════════════════════════╗
        ║  ci-success  ·  the one required check on main   ║
        ║  runs always · a skipped gate counts as pass     ║
        ║  fails only if a gate fails or is cancelled      ║
        ╚═════════════════════════════════════════════════╝
```

A docs-only PR skips every Rust job (they report `skipped`, which
`ci-success` treats as pass), so it goes green in seconds. A pure-`.rs` change
skips the three `cargo` metadata jobs.

## Not on the PR gate

Three things run outside the merge gate, so they never slow a PR:

```text
  job               role       OS          runs on
  ────────────────  ─────────  ──────────  ─────────────────────────────────
  cross-os          signal     mac · win   non-PR, when rust changed, or a tag
  Analyze (Rust)    baseline   ubuntu      push→main + weekly, never on a PR
  release·package   release    all         tag push (validates in plan on PRs)
```

`cross-os` is a signal: a red run does not block a release. `Analyze (Rust)`
(CodeQL) is a post-merge baseline, so it is deliberately **not** a required
check.

## Every gate at a glance

| Job | Guards | Filter | OS | Status |
| --- | --- | --- | --- | --- |
| `rustfmt` | formatting (`cargo fmt`) | rust | ubuntu | required |
| `clippy` | `-D warnings`, panic-free core, `too_many_lines` | rust | ubuntu | required |
| `test` | `cargo nextest run --workspace` | rust | ubuntu | required |
| `cargo-deny` | licences, RUSTSEC, unknown sources | cargo | ubuntu | required |
| `cargo-machete` | declared-but-unused deps | cargo | ubuntu | required |
| `dependency-rule` | hexagonal crate dep rule | cargo | ubuntu | required |
| `actionlint` | valid, shellcheck-clean workflow YAML | workflows | ubuntu | required |
| `markdownlint` | 80-col Markdown prose | markdown | ubuntu | required |
| `ci-success` | aggregates the eight gates | always | ubuntu | the check |
| `cross-os` | clippy + tests on mac and win | non-PR / tag | mac·win | signal |
| `Analyze (Rust)` | CodeQL taint / cross-function SAST | push→main | ubuntu | baseline |
| `release`·`package` | archives, installers, GitHub Release | tag | all | release |

## What runs when

| Event | Runs |
| --- | --- |
| Pull request → `main` | `changes` + gated ubuntu jobs → `ci-success`. `cross-os` and CodeQL skipped; release validates in plan mode. |
| Merge / push → `main` | the PR gates re-run, plus `cross-os` and CodeQL `Analyze (Rust)`. |
| Release tag | `release` + `package` build and publish; `cross-os` is forced on. CodeQL does not run on tags. |
| Weekly · Mon 07:00 UTC | CodeQL only; catches query-pack drift on code already on `main`. |

## CodeQL query suite

CodeQL stays on the full `security-and-quality` suite, not the leaner
`security-extended`. Trimming was weighed for speed, but with CodeQL off the PR
path its runtime no longer blocks anyone, so shrinking would only drop the
maintainability and quality queries for no wall-clock gain. See
[`CI.md`](CI.md) §3.

## Mirror it locally before you push

```bash
# the ubuntu merge gate, in seconds
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace           # CI uses cargo nextest run
cargo deny check
cargo machete
just check-deps                  # hexagonal dependency rule
markdownlint-cli2                # uses .markdownlint-cli2.jsonc
```

Branch protection requires `ci-success`, not `Analyze (Rust)` or `cross-os`.
