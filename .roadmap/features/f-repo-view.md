+++
id = "F-repo-view"
type = "feature"
area = ["sidebar", "sessions"]
status = "todo"
target = ["Could"]
+++

A per-repository surface to browse and manage one repo's sessions.

A per-repository surface to browse and *manage* one git repo's sessions
(rename, archive, launch, compare) instead of only the flat `project_path`
grouping in the sidebar. Filed as **#148**, `needs-design`: git awareness
exists today only to *normalize* paths (worktrees collapse onto their main
repo), so "the sessions of this repo" is not a concept `core` holds. Design
before scope — what the surface is (pane, tab, modal) decides most of the cost
