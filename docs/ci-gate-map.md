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

  changes · dorny/paths-filter — one boolean per file category
  ───────────────────────────────────────────────────────────────────
    rust       →  rustfmt · clippy · test · portable · intra-crate-arch
    cargo      →  cargo-deny · cargo-machete · dependency-rule
    markdown   →  markdownlint
    workflows  →  actionlint
    roadmap    →  roadmap
    book       →  mdbook
  ───────────────────────────────────────────────────────────────────
                                 ▼
  ╔═════════════════════════════════════════════════════════════════╗
  ║  ci-success  ·  the one required check on main                  ║
  ║  runs always · a skipped gate counts as pass                    ║
  ║  fails only if a gate fails or is cancelled                     ║
  ╚═════════════════════════════════════════════════════════════════╝
```

A docs-only PR skips every Rust job (they report `skipped`, which
`ci-success` treats as pass), so it goes green in seconds — though one touching
`docs/src/**` still fires `mdbook`. A pure-`.rs` change skips the three `cargo`
metadata jobs.

## Not on the PR gate

Four things run outside the merge gate, so they never slow a PR:

```text
  job               role       OS          runs on
  ────────────────  ─────────  ──────────  ─────────────────────────────────
  cross-os          signal     mac · win   non-PR, when rust changed, or a tag
  Analyze (Rust)    baseline   ubuntu      push→main + weekly, never on a PR
  release·package   release    all         tag push (validates in plan on PRs)
  docs-deploy       publish    ubuntu      push→main under docs/ (book → Pages)
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
| `portable` | clippy + tests for every crate but the GUI, **on Windows** | rust | win | required |
| `intra-crate-arch` | module boundaries + OS-cfg containment; length report | rust | ubuntu | required |
| `cargo-deny` | licences, RUSTSEC, unknown sources | cargo | ubuntu | required |
| `cargo-machete` | declared-but-unused deps | cargo | ubuntu | required |
| `dependency-rule` | hexagonal crate dep rule | cargo | ubuntu | required |
| `actionlint` | valid, shellcheck-clean workflow YAML | workflows | ubuntu | required |
| `markdownlint` | 80-col Markdown prose | markdown | ubuntu | required |
| `roadmap` | `.roadmap/` schema; `ROADMAP.md` still matches its source | roadmap | ubuntu | required |
| `mdbook` | the book builds; every `SUMMARY.md` link resolves | book | ubuntu | required |
| `ci-success` | aggregates the twelve gates | always | ubuntu | the check |
| `cross-os` | clippy + tests on mac and win | non-PR / tag | mac·win | signal |
| `Analyze (Rust)` | CodeQL taint / cross-function SAST | push→main | ubuntu | baseline |
| `release`·`package` | archives, installers, GitHub Release | tag | all | release |
| `docs-deploy` | publishes the book to Pages | push→main, docs | ubuntu | publish |

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
just check-arch                  # module boundaries + OS-cfg containment
markdownlint-cli2                # uses .markdownlint-cli2.jsonc
just roadmap                     # recompile ROADMAP.md, then validate
just docs                        # the book builds (mdbook)
```

`portable` has no local mirror — it is a *Windows* run.

Branch protection requires `ci-success`, not `Analyze (Rust)` or `cross-os`.
