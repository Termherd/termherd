# Tooling traps

Traps in the tools around the code — mutation testing, CI, git, lint, the
board. Each was paid for once. The project skills point at the section
they need; read the rest before reaching for the tool.

## Mutation testing with cargo mutants

**`cargo mutants` reads back as a trap.** Two lessons paid for during the
screenshot rung, both about mistaking an in-progress run for a finished one:

- `mutants.out/outcomes.json` is rewritten *as the run proceeds*. Reading it
  mid-run yields a plausible partial tally — "16 mutants, 0 missed" when 141
  were queued — that looks exactly like a result. Only quote a run that has
  printed its `N mutants tested in Xm` summary line.
- Deleting or reusing `mutants.out/` while a run is live kills it
  (`File exists (os error 17)`). Pass `--output "$TMPDIR/mutants-<topic>"` so a
  run cannot collide with the working tree or with another run.

**A test-only temp path is shared state.** The private per-session directories
`launch.rs` builds are named from the session id, so two runs of the same test
collide and a leftover directory silently changes what the next run observes —
one `cargo mutants` survivor was a stale `termherd-shell-97/` from an earlier
failing run, not a real gap. A test that writes under `std::env::temp_dir()`
must clear its target first and remove it after.

**A mutation run leaves proptest seeds behind.** A mutant that fails a
`proptest!` makes it write the shrunk case to `proptest-regressions/`,
untracked, beside the seeds that guard real failures. The file records a
failure of the mutant, not of the code, so delete it rather than commit it.
An `--in-place` run on #496 left one under `crates/core/`.

**Timeouts dominate the wall clock.** Do not force `--timeout`; the
auto-derived value (baseline × a factor) is far tighter. And keep test fixtures
small — one 3000×2000 test frame took a run from 8 minutes to 22, because a
mutant that makes an inner loop quadratic then burns the whole budget.

**Prefer `--in-diff` over whole files.** `git diff > d.patch && cargo mutants
--in-diff d.patch` mutates only the lines the session changed — 30–100 mutants
in a couple of minutes, against several hundred for the files they live in.
Cheap enough to re-run after every fix, which is how the keyboard rung caught
four survivors that a single end-of-session pass would have reported too late
to act on.

`--re` is no substitute: it does not filter the *delete field* mutants. A
pass on #385 ran `--re 'quit_prompt$|quit_confirms$'` (cargo-mutants
27.0.0), and `--list` still queued `delete field size … in run` and
thirty-odd other struct-field deletions across `shell/`, so the run ended
"33 missed, 12 caught" for four mutants that were all caught. Read the
outcome by grepping the function name in `caught.txt` / `missed.txt`, never
from the summary line.

**"All caught" says nothing about a mutant never generated.** On #284,
`--in-diff` reported "2 mutants tested: 2 caught", yet `create_new(true)` →
`create(true)`, the line the fix exists for, survived a hand mutation:
cargo-mutants does not mutate a literal argument or delete a statement.
Hand-mutate the line a security fix rests on.

## Waiting on CI

**Wait on CI with `gh pr checks <N> --watch`, not a parsing loop.** A loop
that read the state as the second whitespace column of `gh pr checks` saw
`(windows)` for `portable (windows)`, found no `pending`, and exited while
`portable` was still running: the one slow gate, read as finished by a probe
that broke on its name. `--watch` returns only when every check completes; read
`ci-success` afterwards.

## Filing an issue on the board

**Read GitHub ids; there is no way to recall them.** Filing an issue here is
half of filing it — `AGENTS.md`'s bullet *Filing an issue is not the end of
filing it* requires `Horizon` and `Effort` on every issue, plus `Class` on a
Feature or `Severity` on a Bug (a Task carries neither), all in the same pass.
The per-type table lives in `scripts/board_check.py`'s docstring, which is what
enforces it. Every one of those is id-addressed, and a guessed id fails
misleadingly: an invented
`issueTypeId` answers `Issue type cannot be found in this organization`, which
reads as "no such type" rather than "wrong id". Fetch all three:

```bash
gh api graphql -f query='{ organization(login: "Termherd") {
  issueTypes(first: 10) { nodes { id name } } } }'   # Task / Bug / Feature
gh project field-list 1 --owner Termherd --format json   # fields + option ids
gh project item-list 1 --owner Termherd --format json --limit 200  # item ids
```

Then `updateIssue(input: {id, issueTypeId})` over `gh api graphql` for the
type — the native type is *not* named by that bullet, but an unclassified type
is as invisible as an empty field — and `gh project item-edit` per board field.
`just board-check` is what tells you it took, and it needs the sandbox
disabled: inside it, the recipe fails on TLS and reports *unchecked*, exit 2,
which is not the same as clean. On Windows the recipe runs `py -3`, since
`python3` there is usually the Microsoft Store stub, and the script decodes
`gh` output as UTF-8 itself: before #428, the locale codec failed on it and
the board read as *unreachable*, which looks like a network fault.

## Editing files from a script

**A scripted edit can succeed and still eat the file.** The warning everyone
repeats is the substitution that matches nothing and looks like a success; the
one that cost an amend here was the opposite. Resolving a `.wrap.md` conflict
with a Python slice — `s[:start] + resolved + after` — rebuilt the region
correctly and **truncated everything past it**: one note cut mid-sentence, the
next gone entirely. `markdownlint` reported exactly one problem — MD047, a
missing trailing newline. That is the truncation's *side effect*, not the
truncation: a linter can tell you a file is malformed, never that content is
absent, so the one signal it gave pointed at a one-character fix while two
notes were gone. Read the diff's shape before committing: `git diff --stat`
and a count of removed lines against the number you meant to remove. Six
removed where six were retracted is an answer; forty is a truncation wearing
a nearly clean lint.

That check is also the only one that catches the *loud* failure, which is
counter-intuitive enough to have cost a bundled commit. A `python3 - <<'EOF'`
block that raises exits non-zero and says so — and the `git add` on the **next
line of the same Bash call** runs anyway, because a heredoc is a command, not a
guard. Splitting one file's two ideas into two commits is exactly where that
bites: the assertion caught the reflowed target string, the shell ignored it,
and both ideas were staged for one commit. Chaining with `&&` closes that
variant, and only that one — the silent no-match above still exits 0 and
proceeds. Whichever way it fails, the staged shape is what tells you.

## Git from an agent session

**`git cherry-pick --abort` is not an undo for the conflict you were fixing.**
It discards the whole in-progress resolution, including the parts that applied
cleanly and the edits you made by hand. Reached for after a `--continue` that
printed an editor hint instead of committing, it cost the entire merge a second
time. Keep `--abort` for actually abandoning the pick.

For a **single-commit** pick — the case here, and the usual one when rescuing
one orphan commit — `git commit` finishes it and `cherry-pick -n` avoids the
`--continue` step altogether. Neither shortcut generalises: `-n` suppresses the
commit, not the sequencer, so a conflict partway through `cherry-pick -n A B C`
still halts with `.git/sequencer` holding the rest, and only `--continue` walks
it. Know which of the two you are in before reaching for either.

**A rebase from an agent session stalls at the first signature, and the way
out is not `--abort`.** GPG's pinentry cannot reach the agent's shell, so
`git rebase origin/main` applies the first pick cleanly and then fails at
`commit` with `signing failed: Operation timed out`, leaving the pick's changes
staged and the same pick rescheduled in the todo. The recovery: the user runs
`git commit -S -C <original sha>` (message and author reused) and then
`rebase --continue`; git drops the rescheduled pick as "patch contents already
upstream" and moves to the next one. A conflict there is resolved as usual, but
the `--continue` again needs the user's terminal for the signature — plan for
two hand-offs per rebase, not one. Seen rebasing #308 after six merges had
landed under it.

**A pull request whose branch is checked out in another worktree is updated
from a scratch branch.** Git refuses to check the branch out twice, and a
worktree-isolated session refused `git -C` into the other one. From your own
worktree: `git switch -c merge-<N> origin/<branch>`, merge `origin/main`,
resolve, then `git push origin merge-<N>:<branch>` and delete `merge-<N>`. The
other worktree's local branch is then behind its upstream and needs a pull
before anyone works there. Before syncing a branch again after a CI wait, read
`gh pr view <N> --json state`: on 2026-11-08 #487 had been merged during the
wait, and a second, local merge of `main` went into a branch that no longer
had a pull request.

## Markdown lint

**`markdownlint-cli2`'s summary counts files with findings, not files linted.**
A clean run ends `Summary: 0 issues in 0 files`, which reads as "linted
nothing". Coverage is the line above it, `Linting: N files`. The repo config
adds its own `**/*.md` globs to whatever path you pass, so N is ~100 from the
root or from any worktree. In the #102 pass that line got cut off by a
`tail -1`, the summary was read as a vacuous pass, and a wrong "markdownlint
lints nothing from a worktree" claim went to the user and nearly into this
file. Read the
`Linting:` line, or inject a known violation once. A single long token will not
do, because MD013 skips what cannot wrap: use a long line with spaces. Also do
not substitute `npx markdownlint-cli` (v1) for `markdownlint-cli2`: v1 ignores
`.markdownlint-cli2.jsonc` unless passed `-c`, so it fails on existing table
rows (MD013) and `<kbd>` HTML (MD033) in `ROADMAP.md`, `README.md`, and
`docs/CI.md` that CI's `DavidAnson/markdownlint-cli2-action` explicitly allows.

## On the Windows machine

**On the Windows machine, three gates cannot run and one lies.**
`markdownlint-cli2`, `mdbook` (`just docs`) and `roadmark` (`just roadmap`) are
not installed, and `npx` is not on the path, so the book, the Markdown lint and
the `ROADMAP.md` regeneration are CI's to verify. `just check-deps` is the
dangerous one: without `jq` it prints `jq: command not found` twice, then
`OK: all 0 crates respect the hexagonal dependency rule` and exits 0. Read the
crate count, not the OK (#408). A pass here that edits `.roadmap/` leaves
CI's `roadmap` job failing on the stale artifact until `ROADMAP.md` is
regenerated. The user's `just roadmap` is one way; the other is the failed
job's own log, which prints the `diff -u` between the committed file and the
one it compiled. Applying that hunk is the regeneration, byte for byte, and
turned the job green on #512.

**A Python script on this machine writes CRLF.** `open(p, 'w')` translates
every `\n` to `\r\n` on Windows, so a scripted edit of an LF file rewrites
every line, and a later `assert old in s` written with `\n` stops matching.
Read and write with `newline=''`.
