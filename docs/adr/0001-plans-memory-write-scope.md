# ADR 0001 — Narrow write-scope for in-app plan & memory editing

- Status: accepted
- Date: 2026-06-21
- Feature: `F-plans-memory` (editing slice)

## Context

`termherd` lets the user browse their Claude plans and memory: the global
`~/.claude/CLAUDE.md`, plan files under `~/.claude/plans/`, and each project's
own `CLAUDE.md`. The browse/read slice shipped first; the point of the Must is
to let the user *edit* these in-app.

Editing means writing. Two constraints collide:

- PRD §"Security" scopes filesystem access to `~/.claude` **read-only** and
  `~/.termherd` read/write, and explicitly forbids `~/.claude/ide` writes in v1.
- A live Claude process may write `CLAUDE.md` or a plan file at any moment, so
  a naive last-writer-wins save risks silently clobbering its changes.

## Decision

Relax the read-only rule for `~/.claude` **narrowly**, and enforce the relaxed
boundary in pure, testable code.

1. **Allow-list (the security boundary).** In-app writes may target only:
   - `~/.claude/CLAUDE.md` (global memory);
   - `~/.claude/plans/*.md` (plan files, direct children only);
   - any project `CLAUDE.md` (outside `~/.claude`, normal repo scope).

   Everything else under `~/.claude` is denied — notably the session JSONL in
   `~/.claude/projects/**` and anything under `~/.claude/ide/**`, plus nested
   plans and non-`.md` files.

2. **The rule lives in `core`.** The allow-list is a pure predicate
   (`core::docscope::is_writable`) with no I/O, exhaustively unit- and
   property-tested. The app adapter (`crates/app/src/docs.rs`) performs the
   actual write only after the predicate says yes — the boundary is not an
   ad-hoc check scattered in the GUI.

3. **Atomic save.** Writes go to a temp file in the same directory followed by
   a rename, so an interrupted write never truncates the original.

4. **Concurrency guard.** The file's mtime is captured at open. On save,
   `core::docscope::decide_save` compares it to the on-disk mtime; a file that
   changed since open yields a `Conflict` and the save is refused with a
   warning rather than overwriting a concurrent writer's changes.

## Consequences

- ✅ The `F-plans-memory` Must ships in-app editing of plans and memory.
- ✅ The write-scope is a tested `core` predicate, narrower than "all of
  `~/.claude`" — the session tree and `ide/` stay read-only.
- ⚠️ This is a deliberate, documented relaxation of the PRD's read-only
  `~/.claude` rule. The PRD security note links here.
- 🔁 If Claude starts locking these files, or concurrent-edit collisions show
  up in practice, upgrade the mtime warning to a real lock.
