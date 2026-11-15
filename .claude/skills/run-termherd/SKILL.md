---
name: run-termherd
description: Launch a termherd build to see a change working, beside the user's own running termherd, and observe it through tracing, a capture or the MCP bridge. Use when asked to run, start or screenshot termherd, to check a GUI or OS-level behaviour no headless test reaches (quit flow, window, canvas), or to test a worktree build.
---

# Run termherd

The full reference is `docs/agents/running.md`; read it before the first
launch in a session. The steps that matter:

1. Launch with `just dev-instance`. The user's release termherd usually
   hosts this session and holds the single-instance lock; the recipe gives
   the build its own temp dir and data dir (or it refuses to start, or
   repaints the user's app with its settings), keeps the real shell home, and
   points `CLAUDE_CONFIG_DIR` at a generated fixture: three invented repos,
   nine sessions dated from now. It prints its root on stderr
   (`dev instance: …`). `TERMHERD_MCP_ATTACH=1 just dev-instance` publishes
   the bridge to `<root>/data/bridge.json`.

   Edit the fixture (`crates/scan/examples/dev_fixture/fixture.rs`) when a
   check needs other sessions; never point a build at the user's `~/.claude`
   for a screenshot. By hand, on Windows: `TMP`, not `TMPDIR`, as a Windows
   path, or the build takes the user's lock and exits with code 2. Never
   isolate with `USERPROFILE` or `HOME`: that hides `~/.claude` and changes
   the shells' home.
2. Launch it in the background and read its `tracing` output; add `info!` at
   the seam you are verifying, never `println!`.
3. A build an agent launches inherits this session's `CLAUDE*` variables, so
   a Claude started inside it writes no session file. Anything reading
   Claude's session files is verified from a build the user starts.
4. A branch made before the lock moved does not see a newer build: build the
   merged tree (`git merge --no-commit origin/main`, build,
   `git merge --abort`).
5. To observe: `⌘⇧S` / `Ctrl+Shift+S` writes a JSON + PNG capture to
   `captures/` in the data dir (the latest is the highest name); the MCP bridge
   reads a live app — see the `drive-termherd` skill.
