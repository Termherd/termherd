+++
id = "F-mcp-agent-loop"
type = "feature"
area = ["mcp", "sessions"]
status = "todo"
target = ["Could"]
+++

The composed prompt→wait→read over any session, shell or Claude.

The composed prompt→wait→read over **any** session, shell or Claude: the
primitive shipped as `run_in_session` (#194) and is kind-agnostic, so what is
left is the one-round-trip composition, the guards, and an opt-in scoped to the
nested-Claude case only. Depends on #195
