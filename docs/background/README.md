# Background — why termherd exists

These four documents are **dated snapshots from 2026-05-27**. They are the
analysis that produced the restart decision and informed the v0 scope. They
are not living docs — the living docs are
[`../PRD.md`](../PRD.md) and [`../ARCHITECTURE.md`](../ARCHITECTURE.md). Treat
this folder as the project's *reasoning archive*.

| Doc | What it answers |
| --- | --------------- |
| [`assessment.md`](assessment.md) | Why restart — vs forking or upstream PRs to `doctly/switchboard`. The strategy framing (Alt 1 vs Alt 2) and the "fixing quality is more hazardous than building fresh" heuristic that anchors everything else. |
| [`feature-sizing.md`](feature-sizing.md) | T-shirt sizes per feature, MoSCoW, and the Continue-vs-Restart estimate (cold rebuild ~86 PD vs lifting MIT modules ~69 PD vs the author's actual ~55 PD). Appendix A is the git-log extrapolation. |
| [`electron-architecture.md`](electron-architecture.md) | The as-built architecture of the Electron app (the *reference implementation* you'll often want to consult during the restart — module map, data flows, the IDE/MCP protocol, the known structural risks). |
| [`electron-nfrs.md`](electron-nfrs.md) | The Electron app's NFR scorecard — packaging, signing, auto-update, CI, security, performance. Source of truth for "what gap is `Q<n>` fixing?" cross-references in `PRD.md` §4. |

## Notes on framing

- `assessment.md` predates the rename to termherd and is written from the
  perspective of evaluating `doctly/switchboard` (which the author did not own).
  Its conclusions — restart, MIT-lift the domain modules, defer the IDE — are
  preserved verbatim because they are *the reason this repo exists*.
- The original PRD (`../PRD.md`) and architecture (`../ARCHITECTURE.md`) were
  also drafted on 2026-05-27 and have since been revised (rev. 2) when the
  MoSCoW reshuffled the IDE/diff into the Unsure bucket. The numbers in
  `feature-sizing.md` reflect the pre-rev.2 estimate and are still useful as
  the *budget anchor*; the post-rev.2 milestone plan lives in `../PRD.md` §9.
