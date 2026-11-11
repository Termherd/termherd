+++
id = "F-antigravity-sessions"
type = "feature"
area = ["sessions", "terminal"]
status = "todo"
target = ["Should"]
+++

Discover, parse and launch Antigravity (`agy`) sessions alongside Claude's.

Support Antigravity (`agy`) sessions in TermHerd (Should). Both issues sit
under the provider design #387 and follow its capability model, mirroring the
Codex slices #389–#391; none starts before that design settles.

- **Discovery and transcripts (#160, merged with the former #161)**: scan
  `~/.gemini/antigravity-cli/brain/` for UUID directories, parse CWD and
  metadata incrementally (via `ScanCache`), and a pure JSONL transcript
  parser for summary prompts, message count and indexed text. The `agy`
  paths were last verified in July 2026.
- **Launch and UI (#162)**: split into a provider-neutral `Launch` shared with
  Codex (#549), the `agy` / `agy --conversation <id>` command line (#550), a
  launch button and provider badge (#551), and an activity and metadata spike
  (#552).
