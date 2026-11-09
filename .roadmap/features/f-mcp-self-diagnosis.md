+++
id = "F-mcp-self-diagnosis"
type = "feature"
area = ["mcp"]
status = "todo"
target = ["Backlog"]
+++

Ship with the bridge what an agent now learns from this repo's docs.

Design-first; nothing here is scoped. The review of the agent context files
on 2026-11-07 moved the calling rules into the server's instructions, and
left four things an agent outside this repo still cannot learn or do from the
bridge alone; a fifth, the app's own log, was filed since:

- **Guidance as MCP resources or prompts.** The diagnosis recipes live only
  in a project skill (`drive-termherd`), so a client
  launched from another repo never reads them. The stdio server already
  publishes `termherd://keys/schema`; a guide resource, or MCP prompts per
  recipe, would travel the same way.
- **The PTY bytes a press produced.** The ground truth for a keyboard defect
  is a fresh shell running `stty raw -echo` and `dd | xxd` while
  [F-mcp-keys](#f-mcp-keys) presses. A tool that answers the bytes written to
  a session would replace the recipe and the second shell.
- **The clipboard.** Checking what `copy` wrote needs `pbcopy` / `pbpaste`
  outside the sandbox, on macOS only. A read of termherd's own last
  clipboard write would make the check cross-platform and sandbox-free.
- **A wait that cannot answer on the old status.** `wait_for_status` still
  answers at once on the status from before a prompt, which is why every
  caller is told to wait for `busy` first; `prompt_in_session` fixed it for
  its own loop only (#435). A wait for the next transition would make the
  rule unnecessary. Related open defect: #436.
- **The app's own log** (#483, filed). A warning exists only on the stderr of
  whoever launched termherd, which for a release build is no terminal at all
  (on Windows, a file in the data dir since #508), so "this warning appears,
  or no longer does" cannot be checked over the bridge.
