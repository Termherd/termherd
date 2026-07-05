# CI gates — reference & runbook

How TermHerd's continuous integration is wired: every automated gate, what
it protects, when it runs, and how to reproduce it locally before you push.

TermHerd exists to fix four quality gaps (god-object, races, silent catches,
untestable design) **by construction** (see `AGENTS.md`, `docs/PRD.md` §4).
CI is half of "by construction": the rules below are enforced by a machine on
every change, not by reviewer memory. They cluster on three axes:

- **Correctness & safety** — `fmt`, `clippy`, `test`, `codeql`.
- **Structure & maintainability** — `too_many_lines` (inside `clippy`),
  `dependency-rule`.
- **Supply-chain hygiene** — `cargo-deny`, `cargo-machete`, SHA-pinned
  actions.

Two more gates keep the meta-layer honest: `actionlint` (the workflows
themselves) and `markdownlint` (the prose).

---

## 1. At a glance — every gate

| Gate | Workflow · job | Protects | Runs on | OS | Blocking |
| --- | --- | --- | --- | --- | --- |
| Formatting | `ci` · `rustfmt` | Consistent layout (`cargo fmt`) | PR, push→main | ubuntu | yes |
| Lint + complexity | `ci` · `clippy` | Clippy `-D warnings`; `unwrap`/`expect`/`panic` (core/claude), `too_many_lines`, `todo`/`unimplemented` | PR, push→main | ubuntu | yes |
| Tests | `ci` · `test` | `cargo nextest run --workspace` | PR, push→main | ubuntu | yes |
| Cross-OS clippy + tests | `ci` · `cross-os` | Same clippy + `nextest` on macOS & Windows | push→main, tag (skipped on PR) | mac · win | signal |
| Licenses / CVEs / sources | `ci` · `cargo-deny` | Disallowed licences, RUSTSEC advisories, unknown registries | PR, push→main | ubuntu | yes |
| Unused deps | `ci` · `cargo-machete` | Declared-but-unused dependencies | PR, push→main | ubuntu | yes |
| Architecture | `ci` · `dependency-rule` | Hexagonal crate dep rule (deps point inward) | PR, push→main | ubuntu | yes |
| Workflow lint | `ci` · `actionlint` | Valid, shellcheck-clean workflow YAML | PR, push→main | ubuntu | yes |
| Docs lint | `ci` · `markdownlint` | 80-col Markdown prose | PR, push→main | ubuntu | yes |
| Merge gate | `ci` · `ci-success` | Aggregates every PR job into one required check | PR, push→main | ubuntu | yes (the one required check) |
| SAST | `codeql` · `Analyze (Rust)` | Taint / cross-function security & quality | PR, push→main, weekly | ubuntu | yes |
| CLI release | `release` · `plan…announce` | Build archives + curl\|sh / PowerShell installers, cut the GitHub Release | tag push (validates on PR) | mac · win · ubuntu | release-time |
| Desktop installers | `package` · `package` | `.app`/`.dmg`, NSIS `.exe`, `.deb`/`.AppImage`, attached to the Release | tag push | mac · win · ubuntu | release-time |

"Blocking" = a red run blocks merge (PR/CI gates) or blocks the release
(release-time). "signal" = it runs and reports red/green but does **not** block
merge (see `cross-os`). The `main` branch-protection rule requires exactly one
check — `ci-success` — which fans in every per-PR job; the rest are still
visible on the PR, and `codeql`'s `Analyze (Rust)` is required alongside it.

---

## 2. By development stage

The same gates appear at different moments. Read this top-to-bottom — it is
the lifecycle of one change.

### Local (before you push)

Mirror the blocking `ci` gates in seconds; see §5 for the exact commands.
This is the cheapest place to catch a failure — do it before opening a PR.

### Pull request → `main`

Everything fans out in parallel (no inter-job ordering):

- **`ci`** — eight gate jobs (`fmt`, `clippy`, `test`, `cargo-deny`,
  `cargo-machete`, `dependency-rule`, `actionlint`, `markdownlint`), fanned
  into the `ci-success` aggregator. `cross-os` is **skipped** on PRs. Branch
  protection requires only `ci-success` (+ `Analyze (Rust)`).
- **`codeql`** — `Analyze (Rust)`.
- **`release`** — runs in *validation* mode (cargo-dist's `plan`; artifact
  builds are gated off unless configured), so a tag push won't be the first
  time the release pipeline is exercised. It does **not** publish on a PR.

Superseded PR runs are auto-cancelled (a fresh push kills the stale run) for
`ci` and `codeql` — see the `concurrency` block in each workflow.

### Merge / push to `main`

`ci` and `codeql` run again on the merged commit — and here `cross-os` also
runs (clippy + `nextest` on macOS & Windows), giving the post-merge baseline
its cross-platform check. These runs are **never cancelled**: they establish
the default-branch baseline (CI status badge, the CodeQL security baseline in
the Security tab).

### Scheduled (weekly)

`codeql` also runs every **Monday 07:00 UTC** (`cron: '0 7 * * 1'`). This
catches drift in the query packs themselves — a newly-shipped CodeQL query
can flag code already sitting on `main`.

### Release (tag push)

Pushing a tag matching `**[0-9]+.[0-9]+.[0-9]+*` (e.g. `v0.1.0`,
`v0.1.0-prerelease.4`) triggers the two release workflows:

- **`release`** (cargo-dist) — `plan → build-local-artifacts +
  build-global-artifacts → host → announce`. Builds the archives and the
  CLI-style installers (`curl|sh`, PowerShell) and **creates** the GitHub
  Release with notes generated from the changelog.
- **`package`** — builds the **GUI desktop** installers per target and
  **attaches** them to the Release that `release` created (it polls for the
  Release to exist, so the two never race to create it).

A version with a `-prerelease.N` suffix is published as a GitHub
*prerelease*.

`ci` also fires on the tag (its `push.tags` glob mirrors `release.yml`), so
`cross-os` exercises macOS + Windows as the release is cut. It is a **signal**,
not a hard gate: it runs in parallel with `release.yml` and a red `cross-os`
does not stop cargo-dist from publishing (the two are separate workflows and
GitHub has no cross-workflow `needs:`). Catch platform breakage on `main`
before you tag.

---

## 3. By pipeline (workflow groups)

### `ci.yml` — the quality wall

Trigger: `push`→`main`, `push` release tag, `pull_request`→`main`,
`workflow_dispatch`. Eight **independent** gate jobs (no `needs:` between them
— they run concurrently) fan into one aggregator, plus a cross-OS signal:

```text
fmt   clippy   test   cargo-deny   cargo-machete   dependency-rule   actionlint   markdownlint
  └──────┴───────┴──────────┴──────────────┴─────────────┴──────────────┴────────────┘
                              ▼
                         ci-success            ← the one required check for `main`

cross-os (mac · win)   ← skipped on PR; runs on push→main and tags; signal only
```

The eight gate jobs run ubuntu-only. `ci-success` `needs:` all of them and is
the single status check pinned in branch protection, so the required-checks
list stays stable as jobs come and go. `cross-os` carries the macOS + Windows
coverage that `clippy`/`test` used to via a 3-OS matrix — moved off the PR path
so the merge gate stays fast and cheap.

Workspace-wide knobs: `RUSTFLAGS: -D warnings` (so any `warn`-level lint —
including `too_many_lines` — becomes a hard error in CI), and a strict
`permissions: contents: read`.

### `codeql.yml` — static application security testing

Trigger: `push`→`main`, `pull_request`→`main`, weekly cron,
`workflow_dispatch`. One job, `Analyze (Rust)`: CodeQL autobuilds the
workspace, extracts a database, runs the `security-and-quality` suite, and
uploads SARIF to the **Security → Code scanning** tab. It needs
`security-events: write` (the only `ci`/`codeql` job that escalates beyond
`contents: read`). Complements `cargo-deny` (CVE/dependency-side) and
`clippy` (in-tree style + simple soundness) with taint tracking and
cross-function patterns neither can see.

### `release.yml` — CLI artifacts & the GitHub Release (cargo-dist)

Trigger: tag push (and `pull_request`, for validation). Autogenerated by
dist — **do not hand-edit**; regenerate with `dist init` / `dist generate`.
Job graph:

```text
plan ─┬─► build-local-artifacts (per-target matrix) ─┐
      └─► build-global-artifacts ───────────────────►├─► host ─► announce
```

`plan` decides what to build; the `build-*` jobs compile archives + hashes +
installers; `host` uploads and **creates** the Release; `announce` finalizes.

### `package.yml` — GUI desktop installers (cargo-packager)

Trigger: tag push, `workflow_dispatch`. A single matrixed `package` job over
four targets:

| Target | Runner | Formats |
| --- | --- | --- |
| `aarch64-apple-darwin` | macos-14 | `app`, `dmg` |
| `x86_64-apple-darwin` | macos-14 (cross) | `app`, `dmg` |
| `x86_64-unknown-linux-gnu` | ubuntu-22.04 | `deb`, `appimage` |
| `x86_64-pc-windows-msvc` | windows-2022 | `nsis` |

Config lives in `[package.metadata.packager]` in `crates/app/Cargo.toml`.
Bundles are unsigned for now (signing/notarization pending certs, OQ5).

---

## 4. By goal (what each gate is really for)

- **"Does it build and pass?"** → `clippy` (`-D warnings`) and `test`
  (`nextest`) on Linux gate every PR; the `cross-os` job re-runs both on macOS
  and Windows on push→main and release tags, so platform-specific breakage
  surfaces on the default branch (and before a release) without slowing merge.
- **"Is it formatted and readable?"** → `rustfmt`, `markdownlint`.
- **"Is a function getting too complex?"** → `clippy::too_many_lines`
  (threshold 150 in `clippy.toml`), enforced inside the `clippy` job.
- **"Does the architecture still hold?"** → `dependency-rule`
  (`scripts/check-crate-deps.sh`): the hexagonal rule that adapters depend on
  `core`, never the reverse, checked against an allow-list of internal edges.
- **"Are core/claude staying panic-free?"** → `clippy` denies `unwrap_used`,
  `expect_used`, `panic` in those two crates (their `Cargo.toml` lint
  tables); tests may use them (`clippy.toml`).
- **"Is our dependency tree safe and lean?"** → `cargo-deny` (licences,
  RUSTSEC advisories, unknown sources) + `cargo-machete` (unused deps).
- **"Could there be a security bug in our own code?"** → `codeql`.
- **"Are the workflows themselves correct?"** → `actionlint`.

---

## 5. Mirror it locally

The toolchain is pinned to **Rust 1.95.0 / edition 2024**
(`rust-toolchain.toml`); `rustup` installs it automatically in the repo.

| Gate | Local command |
| --- | --- |
| `rustfmt` | `cargo fmt --all --check` |
| `clippy` (+ `too_many_lines`, panic-free) | `cargo clippy --workspace --all-targets -- -D warnings` |
| `test` | `cargo test --workspace` (CI uses `cargo nextest run --workspace`) |
| `cargo-deny` | `cargo deny check` (needs `cargo-deny`) |
| `cargo-machete` | `cargo machete` (needs `cargo-machete`) |
| `dependency-rule` | `just check-deps` (or `./scripts/check-crate-deps.sh`) |
| `markdownlint` | `markdownlint-cli2` (uses `.markdownlint-cli2.jsonc`) |

`actionlint` and `codeql` are not part of the routine local loop — they run
in CI. To pre-empt `actionlint`, run the `actionlint` binary over
`.github/workflows/` if you have it installed.

---

## 6. Invariants every gate relies on

These are project-wide conventions; breaking one tends to break CI in a
confusing way.

- **Actions are SHA-pinned.** Every `uses:` points at a commit SHA with the
  human version in a trailing comment (`# v6.0.2`). Bump the SHA and the
  comment **together**; never use a mutable tag. (`release.yml` is the
  exception — it is dist-generated and pins by tag.)
- **Toolchain in lockstep (Q10).** `1.95.0` appears in `rust-toolchain.toml`,
  `Cargo.toml` `rust-version`, and every `toolchain:` input in the workflows.
  Change all of them at once.
- **Least privilege.** Workflows default to `permissions: contents: read`.
  A job escalates only when it must: `codeql` (`security-events: write`),
  `release` / `package` (`contents: write`).
- **PR runs are disposable, `main` runs are not.** The `concurrency` blocks
  cancel superseded PR runs but never cancel a push to `main`, the weekly
  schedule, or a tag.
- **`-D warnings`.** CI sets `RUSTFLAGS: -D warnings`, so a lint at `warn`
  in a `Cargo.toml` lint table (e.g. `too_many_lines`, `unwrap_used` outside
  core/claude) is advisory locally but **blocking** in CI.

---

## 7. Sanctioned exceptions (and where they live)

Each gate has an escape hatch for genuine, documented cases. They are listed
here so an exception is never a surprise:

- **Advisories** (`deny.toml` → `[advisories].ignore`): `RUSTSEC-2024-0436`
  (`paste`, unmaintained, transitive via iced) and `RUSTSEC-2025-0057`
  (`fxhash`, unmaintained, transitive via display-info). Both are
  unmaintained-only, no known vulnerability.
- **Function length** (`#[allow(clippy::too_many_lines)]` with a rationale):
  `crates/app/src/shell.rs::update` and `crates/app/src/shell/view.rs::sidebar`
  — a flat iced dispatcher and an inline layout tree, both refactor
  candidates rather than relaxations of the global threshold.
- **`unsafe`**: the only sanctioned block is `crates/app/src/macos.rs` (AppKit
  FFI for Cmd+Q), a `cfg`-gated module with a `#![allow(unsafe_code)]` and a
  `// SAFETY:` note per block. See `AGENTS.md` → Quality bar.
- **actionlint shellcheck noise**: `SHELLCHECK_OPTS: --severity=warning` drops
  SC2086/SC2129 info notes that the dist-generated `release.yml` trips and we
  don't own.
- **Windows installer**: `package.yml` builds NSIS only — MSI/WiX rejects the
  non-numeric `-prerelease.N` version suffix.

---

## 8. Changing or adding a gate

1. Add the job to `.github/workflows/ci.yml` (most gates) or a new workflow
   if it has a distinct trigger. Keep it `ubuntu-latest` unless the check is
   genuinely platform-dependent (only clippy + tests run cross-OS, and those
   live in the `cross-os` job — off the PR path; the dependency graph,
   licences, formatting, etc. are platform-independent). If the new job must
   gate merges, add it to `ci-success`'s `needs:` list so branch protection
   picks it up without editing the rule.
2. SHA-pin any new action (version in a trailing comment).
3. Give it a **local mirror**: a `just` recipe or a one-line command, added
   to §5 here and to the `AGENTS.md` "CI gates" block.
4. If it has tunable thresholds or an allow-list, put them in a committed
   config (`clippy.toml`, `deny.toml`, a script) — not inline in the YAML —
   so they are reviewable and reusable locally.
5. Add a row to §1 and a line to the relevant §2 stage.

Design rationale for the structural/supply-chain gates lives in
`brainstorm/20260627-ci-quality-gates.md`.
