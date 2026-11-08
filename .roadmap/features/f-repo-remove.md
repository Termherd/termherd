+++
id = "F-repo-remove"
type = "feature"
area = ["sidebar"]
status = "todo"
target = ["Could"]
+++

Take a project or repository out of the sidebar, durably and explicitly.

The subtraction term of the sidebar's membership rule, whose other half
[F-repo-add](#f-repo-add) introduces:

```text
visible = (discovered ∪ declared) − removed − presentation
```

`removed` is one more flag on the `RepoMeta` overlay, and it means the same
thing from both directions while doing two different things. On a repository
that exists only because it was declared, removing clears the declaration and
it genuinely disappears. On a *discovered* project nothing can be deleted —
`~/.claude` is not ours to write and the next scan brings it back — so removing
is a durable hide. One field covers both; the button's label is what changes.

That is why this is not #265. **#265 is the ephemeral half** — reduce the
sidebar for the duration of a screenshot, gone at the next launch — and the two
were separated precisely so neither has to compromise: #265 weighed ephemeral
against durable and could not pick, because the durable answer was a different
gesture. A `hidden` flag beside `removed` would be two names for one invariant.

Reversibility is the open design point. A *Show removed* toggle mirrors the
archive checkbox that already exists for sessions and is the obvious answer;
whether removal is offered per session as well as per project is a second
question, and the model above is written per project path, so extending it is a
decision rather than a consequence.

[F-repo-prune](#f-repo-prune) is its automated caller: pruning sets this flag
rather than erasing anything, which is what makes an over-eager sweep
recoverable.

**No issue yet, and that is the reason.** Reversibility above is not a detail
to settle while building: a removal with no way back is a different feature
from one with a *Show removed* toggle, and the per-session question changes the
data model rather than the UI. Filed as it stands, the ticket would carry the
decision instead of the work. [F-repo-add](#f-repo-add) shipped the insertion
point — the visibility predicate is written once — so nothing is blocked
meanwhile.
