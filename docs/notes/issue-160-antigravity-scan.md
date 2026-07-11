# Issue #160 — Antigravity Session Discovery & Scan

Extend `FsScanner` to scan Antigravity sessions and group them by project.

## Background

Claude sessions are organized on disk under `~/.claude/projects/` using encoded
project paths. Antigravity sessions are stored in flat directories named by
conversation UUID under `~/.gemini/antigravity-cli/brain/`.

## Requirements

1. Modify `FsScanner` (in `crates/scan`) to scan
   `~/.gemini/antigravity-cli/brain/`.
2. For each subdirectory (conversation UUID):
   - Locate `.system_generated/logs/transcript.jsonl`.
   - Use the Antigravity transcript parser (#161) to extract CWD and metadata.
   - Build a `SessionRecord` where `session_id` is the conversation UUID.
3. Group these records by the derived CWD alongside existing Claude records
   in the sidebar.
4. Integrate the scan with `ScanCache` so that transcript parsing is
   incremental (based on `transcript.jsonl` mtime/size), preventing slow cold
   starts.

## Tests

- Scan a mock `brain/` directory containing Antigravity conversation structures.
- Group Antigravity sessions correctly by derived project path.
- Cache hits/misses behave correctly when files are touched or modified.
