+++
id = "F-fork-detection"
type = "feature"
area = ["sessions"]
status = "todo"
target = ["Should"]
+++

Detect a forked or plan-accepted session — blocked, the signals do not exist.

Fork / plan-accept detection (**blocked**, PRD rev. 7): an investigation of 23
real `~/.claude` sessions found none of the signals the original feature relied
on — `forkedFrom` is never populated, no message `uuid` is shared across
sessions, and there are no sub-120s session transitions. Current Claude Code
appends a resume to the same file (stable `sessionId`), so separate fork files
don't occur. Revisit only if Claude reintroduces forked session files. A
neighbouring but distinct case *does* occur: Claude carries a `customTitle`
across `/clear` into a fresh, unrelated session, so two real files read alike —
handled not by fork detection but by the summary disambiguator (#93), not a
fork
