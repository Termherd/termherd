+++
id = "F-repo-prune"
type = "feature"
area = ["sidebar"]
status = "todo"
target = ["Could"]
+++

Sweep the sidebar for projects whose directory no longer exists.

A maintenance pass, on demand. Nothing today knows whether a `ProjectGroup`'s
path still exists on disk: the scan does `exists()` checks while collapsing
worktrees and deriving a `cwd`, but a group's own liveness is never probed, so
a deleted checkout keeps its row — with its star, its renames, and two launch
buttons that will fail.

The sweep runs **when asked**, not on every scan. An automatic version reads
better and is the dangerous one: an unplugged external drive or an unmounted
network volume would silently take a dozen projects with it. On demand, the
list is proposed and confirmed before anything applies.

It also does not delete. It sets the `removed` flag that
[F-repo-remove](#f-repo-remove) owns, so an over-eager sweep is undone by the
same *Show removed* toggle a manual removal is — the property that makes the
whole thing safe to run without thinking about what is currently mounted.

It concerns both halves of [F-repo-add](#f-repo-add)'s union: a declared
repository whose directory was deleted, and a discovered project whose `cwd`
went away. That symmetry is why it is a feature of its own rather than a
paragraph in either.

**No issue yet: it is blocked on its sibling, not on design.** The sweep sets
[F-repo-remove](#f-repo-remove)'s `removed` flag, which does not exist yet, and
that flag is the whole reason an over-eager prune is recoverable. Building this
first would mean either deleting outright or inventing a second flag to be
merged later — both worse than waiting.
