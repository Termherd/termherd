Source of truth: [`docs/PRD.md`](docs/PRD.md) §5 (MoSCoW). This file is the
short, scannable view; commits land features here when they ship.

## Working order (next up)

**On the board, not in this file.** Execution order lives in
[Project #1](https://github.com/orgs/Termherd/projects/1) as sortable fields —
`Horizon` (Now / Next / Later / Parked / Shipped) crossed with `Class` and
`Effort`. Read the **By horizon** view; within a horizon, take small
Differentiators and Enablers first, and give a Bet a timeboxed probe rather
than a full build. `AGENTS.md` § *Priority scheme* defines the vocabulary.

This block used to be a hand-maintained ranking re-datered at each pass. It
drifted — the 2026-07-12 revision still listed #54 as the next P1 in one
paragraph and as done in the next — because a prose list has no way to be
wrong out loud. The board does: `just board-check` reports every open issue
it hasn't classified.

The MoSCoW buckets below stay tied to PRD §5; they say **whether a feature
exists and why**, never when it gets picked up.

> **Feature-torture pass (2026-06-20).** The seven open/backlog features were
> each pressure-tested; reports live in `.personal/feature-torture/reports/`.
> Verdicts graduated nine slices into issues #51–#60 and the `v0.1.0`
> milestone; the residual design-first items are marked below.

## Reading the buckets

**Must**, **Should** and **Could** are PRD §5's v0 slices (M0–M3, the
daily-driver target).

**Backlog** holds items from the 2026-06-17 feedback gist (`d1d02e5`) that
each need design before they can be scoped — which is why they are here and
not filed as issues. The well-defined items from the same gist are tracked as
issues #18–#29.

**Unsure** is deferred: neither dropped nor committed to.
