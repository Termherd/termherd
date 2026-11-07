---
name: run-termherd
description: Launch a termherd build to see a change working, beside the user's own running termherd, and observe it through tracing, a capture or the MCP bridge. Use when asked to run, start or screenshot termherd, to check a GUI or OS-level behaviour no headless test reaches (quit flow, window, canvas), or to test a worktree build.
---

# Run termherd

The full reference is `docs/agents/running.md`; read it before the first
launch in a session. The steps that matter:

1. The user's release termherd usually hosts this session and holds the
   single-instance lock. Isolate the build's temp dir **and** home, or it
   refuses to start or repaints the user's app with its settings:

   ```bash
   USERPROFILE=$(mktemp -d) TMPDIR=$(mktemp -d) RUST_LOG=info cargo run -p termherd-app
   ```

   On Windows set `TMP` to a directory that exists, never a missing one.
   Copy `~/.claude/projects` into the fake home when the check needs real
   sessions.
2. Launch it in the background and read its `tracing` output; add `info!` at
   the seam you are verifying, never `println!`.
3. A build an agent launches inherits this session's `CLAUDE*` variables, so
   a Claude started inside it writes no session file. Anything reading
   Claude's session files is verified from a build the user starts.
4. A branch made before the lock moved does not see a newer build: build the
   merged tree (`git merge --no-commit origin/main`, build,
   `git merge --abort`).
5. To observe: `⌘⇧S` / `Ctrl+Shift+S` writes a JSON + PNG capture to
   `~/.termherd/captures/` (the latest is the highest name); the MCP bridge
   reads a live app — see the `drive-termherd` skill.
