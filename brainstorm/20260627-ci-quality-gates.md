# Brainstorm: CI quality gates beyond line count

| Field | Value |
| --- | --- |
| **Date** | 2026-06-27 |
| **Duration** | ~16 min (14:32 – 14:48) |
| **Participants** | User + AI Facilitator |
| **Problem shape** | Decision under constraints |

## Framing

Starting question: *should we add a CI guard for file size?* Reframed
during intake — file line-count is a **proxy** for the things actually
feared: complexity, weak domain boundaries, merge-conflict risk. So the
real question is **which proxies deserve a blocking gate**, not "cap
lines."

## Constraints (Step 1)

Any new gate must satisfy, given this repo:

- 3-OS matrix, fast feedback → cheap / cross-platform or ubuntu-only.
- "By construction" mandate → prefer gates that prevent the *cause*
  (god-object, bad boundaries), not just symptoms.
- No false-positive fatigue → tests, generated code, big `match` / data
  tables must not trip it.
- Domain-based (CUPID) → the hexagonal dependency rule is the real
  boundary invariant, more load-bearing than raw lines.
- Supply-chain hygiene → any new action/tool pinned to SHA.

**Already gated** (`ci.yml` + lint profiles): `fmt`, `clippy -D
warnings` (3 OS), `nextest` (3 OS), `cargo deny`, `actionlint`,
`codeql`, `markdownlint`; `unsafe` denied workspace-wide;
`unwrap`/`expect`/`panic` denied in `core`+`claude`;
`todo`/`unimplemented` warned everywhere. Correctness + safety axis is
well covered. The **structural / maintainability** axis is not.

## Candidate signals (Step 2)

A file length · B function length · C cyclomatic/cognitive complexity ·
D dependency-rule violations · E unused deps · F coverage *(dropped —
out of scope)* · G `todo!`/`unimplemented!` → deny · H MSRV · I doc
coverage · J churn×size hotspots · K PR-size warning.

## Outcome

### Decisions

1. **B — function length** (`clippy::too_many_lines`) — **P1**.
   Near-free; enable pedantic lint, tune threshold to dodge
   big-`match` / test noise.
2. **E — unused deps** (`cargo-machete`) — **P1**. One cheap ubuntu
   job.
3. **D Phase 1 — crate-level dependency rule** — **P1**. An
   *architecture fitness function* enforcing the Ports & Adapters
   (Hexagonal) invariant the project already declares: dependencies
   point inward only (`core`→`claude` only; adapters→`core`, never
   reverse; `app` may depend on all). Implementation: script over
   `cargo metadata --no-deps` asserting each crate's internal deps
   match an allow-list. Deterministic, ubuntu-only, no false positives.
4. **D Phase 2 — intra-crate module rules** — **P2**. e.g. ports stay
   in `core::ports`. Needs `cargo-modules` / archtest-style tooling.
5. **C — cognitive complexity** — **P2**. Pedantic lint, needs
   noise-tuning.
6. **A — file length** + **J — churn×size** — **report-only, P3**.
   Good proxies (A surfaced `shell.rs` at 2095 lines; J measures
   merge-risk directly via commits×size) but blocked on a
   report-consumption home (see action items).
7. **Dropped:** G (`warn` keeps future `todo!` meaningful), H (repo
   pins toolchain exactly, claims no MSRV range), K (2 contributors
   already ship small PRs).

### Sequencing logic

- **P1** = config flips + one script (B, E, D-ph1) → land first,
  ~zero ongoing cost.
- **P2** = tooling-dependent (D-ph2, C) → after P1 proves out.
- **P3** = report bucket (A, J) → unblocked only once a quality-report
  home exists.

### Action items

- [ ] Enable `clippy::too_many_lines`, pick threshold, allow in tests
  (B) — P1
- [ ] Add `cargo-machete` CI job, pinned SHA (E) — P1
- [ ] Write `cargo metadata` dependency-rule script + CI job (D-ph1) —
  P1
- [ ] File issues for D-ph1, B, E (graduate from this report per
  AGENTS.md work-tracking rule)
- [ ] Evaluate `cargo-modules` for intra-crate rules (D-ph2) — P2
- [ ] Trial `clippy::cognitive_complexity` threshold (C) — P2
- [ ] **Prereq for A/J:** decide on a quality-report home (CI job
  summary artifact / dashboard) before investing in report-only
  signals — P3

---

## Session Meta-Analysis

- **Duration:** ~16 min
- **Techniques used:** Constraint Mapping (~2m), divergent candidate
  generation (~4m), Impact/Effort (~4m), MoSCoW-style bucketing +
  inline pre-mortem (~6m)
- **Techniques skipped:** SCAMPER (not idea-mutation shape), Six Hats
  (not requested)
- **Adaptations made:** added crate-dependency-rule (D) to the
  candidate set during constraint mapping when the "bad domain
  definition" fear pointed at a *direct* signal the user hadn't
  seeded; reframed file-length from blocking gate to proxy early.
- **Problem shape:** Decision under constraints → held throughout.
- **Convergence point:** Step 3 (Impact/Effort) — the cheap cluster
  (B, E, D-ph1) self-selected; remaining work was bucketing.
- **What worked well:** grounding in `ci.yml` + lint profiles before
  proposing meant zero redundant suggestions; the file-length →
  dependency-rule reframe redirected effort from a noisy proxy to the
  architecture invariant.
- **What could improve:** the report-bucket (A, J) prerequisite — no
  home for non-blocking signals — surfaced late; could have been a
  constraint up front.
- **Session energy:** high — decisive user, fast P1/P2/P3 calls.
- **Recommendation for similar sessions:** for "should we gate X?"
  topics, always enumerate existing gates first and ask "blocking vs
  report" per signal early — it halves the candidate set fast.
