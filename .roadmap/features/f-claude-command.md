+++
id = "F-claude-command"
type = "feature"
area = ["sessions", "keymap"]
status = "todo"
target = ["Could"]
+++

Send a confirmed slash command into an idle Claude session.

One write path for every edit termherd makes to a Claude session (#337): a
closed catalogue (`/rename`, `/color`, `/desktop`), sent only when the session
is idle, behind a confirmation overlay that names the exact line typed. The
overlay is a `KeyboardOwner` rung, so `escape` leaves it. Claude drives the
information termherd shows; termherd sends actions.
