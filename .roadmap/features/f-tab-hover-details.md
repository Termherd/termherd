+++
id = "F-tab-hover-details"
type = "feature"
area = ["workspace", "sessions"]
status = "todo"
target = ["Could"]
+++

The tab hover card shows agent name, model, effort, version and elapsed time.

Extends the single-sourced session card (#76) with what Claude records in the
transcript and in its session file (#344). Needs
[F-session-id-at-launch](#f-session-id-at-launch) and
[F-copy-agent-name](#f-copy-agent-name). Torture report:
`.personal/feature-torture/reports/F-tab-hover-details.md`.

The agent name shipped with [F-copy-agent-name](#f-copy-agent-name) (#339): an
`Agent:` line the tab card carries and the sidebar's does not. Model, effort,
version and elapsed time remain.
