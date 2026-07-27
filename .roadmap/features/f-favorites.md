+++
id = "F-favorites"
type = "feature"
area = ["sidebar"]
status = "done"
target = ["Backlog"]
+++

Favorites in the sidebar — one concept with the session star.

Favorites in the sidebar. **Designed (🧬 split, feature-torture
`F-favorites.md`)**: "star" == "favorite" is one concept. Graduated to #56
(cross-project Favorites section, reusing the shipped session star) and #57
(repo-level favoriting, a `project_path`-keyed overlay in
`~/.termherd/metadata.json`, never `~/.claude`). Both children implemented:
**#57** — a `repos` map in the overlay (`RepoMeta`), a star on each project
header that pins the group to the top, and a flat→wrapped JSON migration;
**#56** — a cross-project "★ Favorites" section at the top of the sidebar
aggregating every starred session (coexists with the in-group pin — the
favourite is a shortcut, not a move). Both merged (#163, #165); the issues are
closed. **Decision — repos in the Favorites section: 👎 killed.** #57's repo
star already surfaces a favourited repo by pinning its group to the top of the
sidebar; a repo row in the section would only duplicate the shipped 🤖/`$`
launch buttons (#23) or navigate to an already-top group, and mixing
non-resumable repos into a "resume a favourite session" list breaks its model.
Favorites stays sessions-only
