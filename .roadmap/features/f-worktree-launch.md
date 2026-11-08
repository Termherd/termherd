+++
id = "F-worktree-launch"
type = "feature"
area = ["sessions", "sidebar"]
status = "done"
target = ["Could"]
+++

Start a Claude session in a fresh git worktree, grouped under its project.

Shipped with #422: the tab menu's *New Claude session in a worktree* (and the
unbound `new-claude-session-in-worktree` action) has termherd run
`git worktree add` itself, under `.claude/worktrees/` of the main checkout,
then open a fresh Claude there. `claude --worktree` was set aside: Claude
changes directory inside its own process, so the pane would stay recorded in
the main checkout and a reopen or a restored layout would resume from the
wrong directory. The sidebar's repo row offers no worktree launch yet.
