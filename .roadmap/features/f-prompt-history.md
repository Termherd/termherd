+++
id = "F-prompt-history"
type = "feature"
area = ["sessions"]
status = "todo"
target = ["Could"]
+++

A read-only panel of the prompts typed in a session, with copy.

Read from the session JSONL through a pure `claude`-crate function (#345); the
filter that tells a typed prompt from tool results and meta entries is the
risk. First slice of [F-jsonl-viewer](#f-jsonl-viewer). Torture report:
`.personal/feature-torture/reports/F-prompt-history.md`.
