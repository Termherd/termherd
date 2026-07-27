+++
id = "F-plans-memory"
type = "feature"
area = ["sidebar"]
status = "done"
target = ["Must"]
+++

Browse and edit plans and `CLAUDE.md` within a narrow, ratified write scope.

Browse/edit plans + `CLAUDE.md` (M3, moved to Must in PRD rev. 6): a sidebar
"Plans & mémoire" section lists `~/.claude/plans/*.md`, the global `CLAUDE.md`
and each project's `CLAUDE.md`, opening one in the main pane (off-thread read
via the `docs` adapter). The editing slice (#53) added in-app editing with a
narrow, ADR-ratified write-scope
([`docs/adr/0001`](docs/adr/0001-plans-memory-write-scope.md)): writes reach
only `~/.claude/CLAUDE.md`, `~/.claude/plans/*.md` and project `CLAUDE.md`,
guarded by a pure `core::docscope` predicate, an mtime concurrency check, and
an atomic temp-then-rename save
