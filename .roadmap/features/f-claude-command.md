+++
id = "F-claude-command"
type = "feature"
area = ["sessions", "keymap"]
status = "done"
target = ["Could"]
+++

Send a confirmed slash command into an idle Claude session.

One write path for every edit termherd makes to a Claude session (#337): a
closed catalogue (`/rename`, `/color`, `/desktop`), sent only when the session
is idle, behind a confirmation overlay that names the exact line typed. The
overlay is a `KeyboardOwner` rung, so `escape` leaves it. Claude drives the
information termherd shows; termherd sends actions.

Shipped (#337): `core::ClaudeCommand` renders the line, and makes a name safe to
type — control characters and line breaks become spaces, invisible formatting is
dropped, a trailing backslash goes, an empty name is refused. The colour is the
closed `ClaudeColor` palette, kept in the `claude` codec so the transcript
reader can share it. `App::claude_command_check` refuses anything but a Claude
launch idle at an empty input prompt — read off the screen by `read_prompt`, so
a draft or an open picker refuses — and is asked again at the send, where a
refusal keeps the prompt open and says why. The confirmation answers `enter` and
`escape` itself, so a synthesised key event reaches both. Two surfaces arm it: a
`send-to-desktop` action, unbound by default, and an MCP `claude_command` tool,
which arms the same prompt instead of typing. Confirming sends Ctrl+U, then the
line, then Enter on its own. A prompt the MCP tool arms ignores a physical Enter
for 600 ms.

Checked against a live Claude Code: the shape of the empty prompt (its `Try "…"`
hint) and of a two-line draft. Not checked: whether the line and its Enter,
written back to back, always submit rather than read as a paste, and whether an
`@` in a name opens the file autocomplete. The rename and colour surfaces that
use this path are #119 and #343.
