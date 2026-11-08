+++
id = "F-repo-add"
type = "feature"
area = ["sidebar", "sessions"]
status = "done"
target = ["Should"]
+++

Declare a repository in the sidebar, before it has any session.

**#279.** A sidebar group is born from a *session*: the scan walks
`~/.claude/projects`, so a repository you have never opened Claude in cannot
appear, and the `$` / 🤖 launch buttons beside a project header — exactly the
gesture you want for it — are out of reach. This adds the missing half.

The change is one line of model and one of consequence. `RepoMeta`, already
keyed by real project path in `~/.termherd/metadata.json` and already written
to grow, gains a `declared` flag orthogonal to its star; and the sidebar stops
being a pure function of the scan:

```text
visible = (discovered ∪ declared) − removed − presentation
```

Only the first two terms are in scope. The predicate is written **once** so
[F-repo-remove](#f-repo-remove) and the ephemeral presentation mode (#265) can
each drop their term into it rather than growing a second visibility rule.

Three details carried the risk. The union is keyed on the path, so a declared
repository that gains its first session stops being empty instead of doubling.
And "declared repositories sort to the top until they have a session" meets the
repo star from [F-favorites](#f-favorites), so the two became one ordered key
(starred → declared-and-empty → recent activity) rather than two competing
sorts of the same list.

**The normalisation was got wrong twice, and the second time is the lesson.**
The plan said a subdirectory resolves to its `repo_root()`; that was corrected
before implementation, on the grounds that `repo_root()` stops at a linked
worktree (whose `.git` is a file) and would file a declared worktree under a
path the scan never produces. The correction composed `repo_root()` *with*
`canonicalize` and the worktree collapse — and both of the added steps were
themselves wrong, for the same reason the first version was, which review
caught: **the scan applies neither**. It keys on the `cwd` the CLI wrote, and
nothing else. So a declared subdirectory was filed at the repository root while
a session started there was filed at the subdirectory (two rows), and
`canonicalize` diverged wherever a symlink stood in the path — on Windows, for
*every* repository, since it yields the `\\?\C:\…` form no transcript contains.

What settled it is a rule, not a fix: there is **one** key function,
`scan::sidebar_key`, and the walk and the declaration both call it. A step that
only one side can apply is not normalisation, it is a second sidebar row. The
cross-check test asserts against the walk's own output, and now on the cases
that actually differ — a subdirectory, and a path through a symlink. Its first
version passed vacuously: it exercised a worktree *root*, where the two rules
coincide, and wrote the already-resolved path into the fixture's `cwd`, putting
the same prefix on both sides of its own assertion.

The Windows half is the sharper half. This entry cited `#239`'s lesson — a
string destined for another grammar takes that grammar's separators — while the
diff carried a fresh instance of it, and a `cargo check --target
x86_64-pc-windows-msvc` was taken for a Windows check. It compiles either way:
the divergence is behavioural, which is exactly what that job cannot see.

Two gestures, converging on a single event so there is one path to test: a `+`
button opening the native folder dialog (`rfd`, `xdg-portal` on Linux so no GTK
is pulled in), and a folder dropped on the window. Neither is exercisable
headless, which is the argument for the convergence — and the reason both log
a `via` field naming the gesture. A declaration made by a drop is
byte-for-byte the one made by the picker, so where no test can watch, the log
is the only thing that can, and an unlabelled line says nothing about which
surface a user actually reached. Dropping a declaration logs too, carrying
whether the row survived on its sessions: "forgotten" and "gone from the
sidebar" are different events and a reader would assume the second.

The agent surface shipped with it: `ProjectSnapshot.declared`, so the ⌘⇧S
capture and the MCP `snapshot` report it from one model, plus `add_repo`
(answering with the key it kept, since the caller may have passed a worktree)
and `forget_repo` (answering whether the row survived on its sessions).
`add_repo` is the first MCP tool to write outside the workspace — into the
overlay, never `settings.json`, so the file-only enumerations in the book stay
true.

One cleanup fell out of it. The snapshot restated the sidebar's filter and
order in its own words under a doc-comment asking the next reader to keep the
two in step; both now go through one `sidebar_row_shown` / `sidebar_row_order`
pair, and a test walks four searches asserting the snapshot's rows *are* the
rendered ones — with distinct mtimes, without which every ordering key is equal
and that assertion cannot fail.

The shared predicate stopped one seam short in four places, which a second
review caught, and the four are one shape: a rule the sidebar applies, restated
or skipped by whoever reads it next. The MCP outcome answered off the *rendered*
list, so a search left in the search box reported a successful `add_repo` as a
failure. The snapshot decided "did this row match" from a count the archive
filter had already thinned, so a declared repo whose only hit was archived
vanished from it while the screen drew it. The fold was masked in the view
alone, so an empty row reported a fold the screen contradicted and went on
storing one invisibly. And the key rule normalised no spelling, so a trailing
slash — which a picker cannot produce and a caller can — was a second permanent
row that `forget_repo` could not reach.

That last one is the same lesson as the first two normalisation mistakes,
arrived at from the opposite side: `canonicalize` was wrong because the scan
cannot apply it, and lexical folding is right because the scan's own producer
already has. The test that missed it built its paths with `PathBuf::join`,
which normalises them away before the assertion ever runs.

The row order is keyed on the **unfiltered** sessions, which review also
caught: taken from the filtered set, typing in the search box reordered the
projects under the cursor, and a repo whose sessions were merely all archived
read as one that never had any and was pinned to the top as freshly added.

Adjacent: [F-repo-view](#f-repo-view) (#148) takes the other end — this is
about a repository *existing* in the sidebar, that one about *viewing* it.
