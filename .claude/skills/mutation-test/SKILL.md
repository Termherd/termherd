---
name: mutation-test
description: Run cargo mutants on the lines a termherd change touched and read the result correctly. Use when evaluating test coverage of a fix or feature, when asked for mutation testing, or before claiming a change is well tested.
---

# Mutation-test a change

The traps behind each step are in `docs/agents/tooling.md` (*Mutation testing
with cargo mutants*).

1. Mutate the diff, not whole files, into a private output directory:

   ```bash
   git diff origin/main > "$TMPDIR/d.patch"
   cargo mutants --in-diff "$TMPDIR/d.patch" --output "$TMPDIR/mutants-<topic>"
   ```

   Do not pass `--timeout`, and keep fixtures small: one large frame turned
   an 8-minute run into 22.
2. Quote a run only after it prints `N mutants tested in …`; the outcomes
   file is rewritten while it runs.
3. Read survivors by grepping the function name in `caught.txt` and
   `missed.txt`. `--re` still queues field-deletion mutants, so its summary
   line overstates the misses.
4. A survivor may be a stale temp directory from an earlier run rather than
   a gap: a test writing under the temp dir clears its target first.
5. cargo-mutants never mutates a literal argument or deletes a statement.
   Hand-mutate the line a security fix rests on.
6. A surviving mutant in a guard nobody can reach is a design smell: remove
   the guard, or make it reachable and test it.
