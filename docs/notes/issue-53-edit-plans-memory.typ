#set page(margin: 2.2cm, numbering: "1")
#set text(font: "New Computer Modern", size: 10.5pt)
#set heading(numbering: "1.")
#show link: set text(fill: blue)

#align(center)[
  #text(size: 16pt, weight: "bold")[Issue \#53 — Edit plans & CLAUDE.md in-app]
  #v(2pt)
  #text(size: 9pt, fill: gray)[Working note · branch `feat/53-edit-plans-memory` · 2026-06-21]
]

#v(6pt)

This note captures the decisions behind issue \#53 (the editing slice of
`F-plans-memory`) so a future reader needs no re-discovery: who owns it, where
the change lives, the test data, and the red test suite that pins the fix down.
It is written *before* the fix — steps 1–6 of the working process.

= Assignment (step 1)

Assigned to `bastien-gallay`. Labels `enhancement`, `P1`; milestone `v0.1.0`.
It is the last unshipped *feature* in the Must list — the other two milestone
items (\#61 Homebrew, \#52 Linux checksums) are release plumbing. Branch
`feat/53-edit-plans-memory` off `main`.

= Root cause — a missing *write path*, not a bug (steps 2, 4)

The read-only browse slice shipped (`crates/app/src/docs.rs`: `discover` +
`read`). Three things block editing:

#table(
  columns: (auto, 1fr, auto),
  inset: 6pt,
  align: (left, left, center),
  table.header([*Layer*], [*Symbol*], [*State*]),
  [`core`], [a write-scope predicate (the security boundary)], [*missing*],
  [`app::docs`], [`write` (atomic, mtime-guarded) + `mtime`], [*missing*],
  [`app::shell`], [`viewing: Option<(String, String)>` — bare tuple, no path/mtime], [too thin],
  [`app::shell::view`], [doc rendered through a read-only `text` widget], [read-only],
)

The load-bearing risk (pre-mortem, from the feature-torture report): a *live
Claude process* may write `CLAUDE.md` or a plan milliseconds after we open it.
Last-writer-wins is silent data loss — hence the mtime guard.

== Tidy-first (Tidy First; before the feature)

Replace `viewing: Option<(String, String)>` with a named `OpenDoc` struct
(`label`, `path`, `mtime`, editor buffer, dirty/status). Behaviour-preserving;
removes the primitive-obsession the edit fields would otherwise widen.

= Design — where the logic lives (dependency rule)

#table(
  columns: (auto, 1fr),
  inset: 6pt,
  align: (left, left),
  table.header([*Crate*], [*Responsibility*]),
  [`core::docscope`], [pure `is_writable(path, claude_home)` + `decide_save(open, on_disk) -> SaveDecision`. No I/O ⇒ exhaustively testable.],
  [`app::docs`], [I/O adapter: stat mtime, call `core` predicate, atomic temp-file + rename, typed `SaveError`.],
  [`app::shell` + `view`], [`OpenDoc` state, `text_editor` widget, Save (⌘S), conflict-warning bar.],
)

The write-scope allow-list (ADR `0001`, ratifying a *narrow* relaxation of PRD
§191):

- ✅ `~/.claude/CLAUDE.md` (global memory)
- ✅ `~/.claude/plans/*.md` (direct children only)
- ✅ project `CLAUDE.md` (outside `~/.claude`, normal repo scope)
- ❌ `~/.claude/projects/**` (session JSONL), `~/.claude/ide/**`, nested plans,
  non-`.md` files, any other file under `~/.claude`

= Test data (step 3) — from the issue spec stub

The issue's "edges" became the test matrix: allow/deny each location; mtime
mismatch → warn before overwrite; out-of-allowlist → rejected by the `core`
predicate; permission/read-only → surfaced, no panic.

= Red test suite (step 6)

All written first and currently *red* (stub impls return `false` / `Proceed`
/ `Ok(())`).

#table(
  columns: (auto, 1fr, auto),
  inset: 6pt,
  align: (left, left, center),
  table.header([*Where*], [*Test*], [*Kind*]),
  [`core::docscope`], [global memory / plan `.md` / project `CLAUDE.md` writable], [UT],
  [`core::docscope`], [session JSONL / `ide` / nested plan / non-`.md` / other denied], [UT],
  [`core::docscope`], [`projects/**` & `ide/**` never writable], [PBT],
  [`core::docscope`], [every direct `plans/*.md` writable; non-`.md` never], [PBT],
  [`core::docscope`], [`decide_save` monotone in on-disk mtime], [PBT],
  [`app::docs`], [atomic write round-trip leaves correct content], [integration],
  [`app::docs`], [out-of-scope path rejected, file untouched], [integration],
  [`app::docs`], [mtime conflict rejected, concurrent content survives], [integration],
  [`app::docs`], [discovered project memory is writable (adjacency regression)], [regression],
)

Run: `cargo test -p termherd-core docscope` · `cargo test -p termherd-app docs`.

= What is left (steps 8–11)

Implement the predicate + atomic write (green), wire the `text_editor` UI,
write ADR `0001` + amend PRD §191 + tick `ROADMAP`, then coverage / mutation
testing and an adversarial CUPID review of the modified methods.
