+++
id = "F-layout-restore"
type = "feature"
area = ["workspace", "sessions"]
status = "todo"
target = ["Could"]
+++

Restore tabs, splits and resumable sessions at launch.

The window position survives a restart (FR12); the workspace inside it does
not. `Workspace` is plain data and sessions already resume (FR4), so this is
saving and replaying the tree.

Issue: #421.
