+++
id = "F-quality-gates"
type = "feature"
area = ["packaging"]
status = "todo"
target = ["Must"]
+++

CI gates for structure and maintainability, beyond fmt/clippy/test/deny.

Intrinsic-quality CI gates beyond the existing fmt/clippy/test/deny set,
targeting the structural/maintainability axis (complexity, domain boundaries,
merge-conflict risk). Scoped from a brainstorm
(`brainstorm/20260627-ci-quality-gates.md`). **P1 — shipped:** function length
(#105), unused deps (#106), and the crate-level dependency rule as an
architecture fitness function enforcing the hexagonal inward-only invariant
(#107) all landed. **P2 follow-ups — shipped** as the intra-crate refactor
cluster #167–#173, now complete: the intra-crate architecture gate
(`intra-crate-arch`) is #173 — a module-boundary check
(`scripts/check-module-boundaries.sh`: leaf modules stay leaves, renderers
don't reach the executor, `core::app` submodules go through the parent
registry), an OS-cfg containment check (`scripts/check-os-cfg-containment.sh`:
compile-time `#[cfg(target_os)]` stays in its audited homes, same spirit as the
`unsafe_code` quarantine), and the report-only file-length signal — fanned into
`ci-success`, mirrored by `just check-arch`. It followed the god-object splits #167
(shell — **shipped**), #168 (terminal/view — **shipped**) and #169 (core `app/`
split + `Sessions` registry + `Sidebar`/`FontState` field-flatten, A1–A4 —
**shipped**), plus the independent adapter splits #170 (scan →
`watch`/`cache`/`derive`/`walk`/`repo` — **shipped**, the seam
`F-antigravity-sessions` #160/#161 build on), #171 (F, json_store —
**shipped**) and #172 (pty →
`input`/`grid`/`events`/`status`/`session`/`kill`/`manager` — **shipped**, the
seam #143 foreground-process detection and #155 vim mouse build on).
Cognitive-complexity (signal C) stays unfiled. **Report-only** (was blocked on
a quality-report home): file length (signal A) now ships **inside #173's gate**
as a job-summary report; churn×size hotspots (signal J) stays unfiled. Dropped:
MSRV check, `todo!`→deny, PR-size warning (rationale in the report)
