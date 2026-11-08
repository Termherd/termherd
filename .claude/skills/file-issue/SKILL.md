---
name: file-issue
description: File a termherd GitHub issue completely — English body, native issue type, area labels, and the board fields Horizon, Effort and Class or Severity — then check it took with just board-check. Use when asked to open, file or report an issue or bug for termherd, or when a session finds a defect worth tracking.
---

# File an issue

Filing is half done until the issue is classified on the board; an
unclassified issue appears in no prioritised view. Board ids are in
`docs/agents/tooling.md` (*Filing an issue on the board*).

1. **Body, in English.** Observed, expected, steps to reproduce; a guess is
   labelled as one. No AI signature. Write it with the Write tool to a file
   under the scratchpad and pass `--body-file <absolute path>`: `$TMPDIR`
   differs inside and outside the sandbox.
2. **Create** with `gh issue create --title … --body-file … --label area:…`
   outside the sandbox (`gh` fails TLS inside it).
3. **Type.** Fetch the type ids, never guess them, then set the type with
   `updateIssue(input: {id, issueTypeId})`. A guessed id answers *cannot be
   found*, which reads as a missing type.
4. **Board fields.** The auto-add puts the issue on the board; fill
   `Horizon`, `Effort`, and `Class` (Feature) or `Severity` (Bug) — a Task
   carries neither. A security bug is 🟡 Minor at most. One literal
   `gh project item-edit` per field; a worktree session refuses loops and
   variables, and GraphQL with inline braces (pass `-F query=@file`).
5. **Check** with `just board-check` outside the sandbox: inside it the
   recipe reports *unchecked* (exit 2), which is not clean.
6. **Link.** A scoped epic's issue is cited in its `.roadmap/features/` file,
   and the file in the issue.
