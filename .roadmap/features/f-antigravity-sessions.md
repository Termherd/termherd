+++
id = "F-antigravity-sessions"
type = "feature"
area = ["sessions", "terminal"]
status = "todo"
target = ["Should"]
+++

Discover, parse and launch Antigravity (`agy`) sessions alongside Claude's.

Support Antigravity (`agy`) sessions in TermHerd (Should):

- **Codec / Parser (#161)**: a pure JSONL transcript parser to extract
  summary prompts, calculate message count, build text content for indexing,
  and derive CWD from tool calls.
- **Session Discovery (#160)**: scan `~/.gemini/antigravity-cli/brain/` for
  UUID directories, parse CWD/metadata incrementally (via `ScanCache`), and
  group them.
- **Process Spawning & UI Integration (#162)**: extend `Launch` and PTY
  spawning to run `agy` and `agy --conversation <id>`, add launch buttons
  to the sidebar, and wire up tab/sidebar display titles.
