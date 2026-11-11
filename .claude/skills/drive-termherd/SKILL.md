---
name: drive-termherd
description: Drive or diagnose a running termherd through its MCP tools (mcp__termherd__*) — type into a session, wait for it, read its screen, press termherd's own keys, click a cell, check what reached the PTY or the clipboard. Use when reproducing a keyboard, mouse, clipboard or status bug in the live app, or when composing the act-wait-observe loop.
---

# Drive termherd over MCP

The server's instructions and each tool's description carry the calling
rules; `docs/agents/mcp.md` holds the design and the open defects. The loop:

1. **Find the target.** `list_sessions`, then address it by `handle`, never
   by the Claude `resume_id`.
2. **Act.** `run_in_session` (a raw write, not a bracketed paste) returns at
   once. For a prompt to Claude, prefer `prompt_in_session`, which waits for
   the prompt to start before waiting for it to finish.
3. **Wait.** `wait_for_status` for `busy` first, then for `idle`: just after
   typing, the status is still the one from before. Never poll `snapshot`.
4. **Observe.** `read_terminal` for text; `screenshot` only for render,
   colour or glyph questions — it weighs hundreds of times a `snapshot`.
   On a Claude pane pass `lines: 80` or more (to `prompt_in_session` too):
   blank rows above the input box can push the reply out of the default 40.

Diagnosing a live defect:

- `press_keys` tests a binding through the live keymap, `run_action` the
  behaviour by action name. An open prompt eats both; `escape` leaves any
  prompt. Read each press's outcome (`ran`, `inert` + `reason`, `overlay`,
  `typed`, `unbound`).
- PTY bytes are the ground truth: in a fresh shell run
  `stty raw -echo; for i in {1..N}; do dd bs=64 count=1 | xxd -p; done`,
  then press.
- Clipboard: `printf marker | pbcopy` (outside the sandbox), act, `pbpaste`.
- `mouse_in_session` answering `forwarded` for a `move` proves the child
  asked for DECSET 1003.
- A synthesised key has `key == text`: Option-composed characters, and keys
  `press_keys` cannot name, need the user's hands.
