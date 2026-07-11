# Issue #161 — Antigravity CLI Transcript Parser

Add a parser for the Antigravity transcript format to extract session metadata.

## Background

Antigravity CLI (`agy`) logs are saved in a JSONL format (`transcript.jsonl`)
under `~/.gemini/antigravity-cli/brain/<conversation-id>/.system_generated/logs/`.
Unlike Claude Code, which registers events with properties like `message`,
`cwd`, and `type`, Antigravity structures logs as conversation steps.

## Requirements

1. Implement a pure codec submodule in `crates/claude` or a new
   `crates/antigravity` crate to parse `transcript.jsonl`.
2. Extract the first `USER_INPUT` step `content` as the `summary` (truncated to
   120 UTF-16 units).
3. Compute `message_count` based on the number of `USER_INPUT` and
   `PLANNER_RESPONSE` steps.
4. Concatenate user/model step content for FTS indexing (`text_content`),
   capped at 8000 UTF-16 units.
5. Extract the project workspace directory (CWD) from the log (e.g., from step
   metadata or `run_command` tool call Cwd arguments) to determine the CWD.

## Tests

- Parse simple transcripts and verify correct summaries/message counts.
- Gracefully handle corrupt or empty lines (skip them).
- Extract the correct CWD from tool calls or step metadata.
