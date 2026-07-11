# Issue #162 — Antigravity Process Spawning & UI Integration

Integrate Antigravity into TermHerd's process lifecycle and UI widgets.

## Background

Users should be able to launch fresh Antigravity (`agy`) sessions and resume
existing ones from the sidebar, just as they do with Claude Code.

## Requirements

1. Update `Launch` enum in `crates/core/src/app.rs`:

   ```rust
   pub enum Launch {
       Shell,
       Claude { resume: Option<String> },
       Antigravity { resume: Option<String> },
   }
   ```

2. In `crates/pty/src/lib.rs`, map `Launch::Antigravity` to typing the launch
   command in the terminal shell:
   - `Launch::Antigravity { resume: None } => "agy\r"`
   - `Launch::Antigravity { resume: Some(id) } => "agy --conversation <id>\r"`
3. Update `crates/app/src/shell.rs` to render a new "Launch Antigravity"
   button/affordance next to the shell `$` and Claude `🤖` buttons in the
   repository list in the sidebar.
4. Wire up the tab & sidebar list: double-clicking or clicking an Antigravity
   session resumes it.
5. Display the correct display title on the tabs and sidebar using the
   session's parsed title/summary.

## Tests

- Spawning a fresh/resumed Antigravity session correctly types `agy`/`agy
  --conversation <id>` into the terminal.
- Launch UI button triggers the correct event and starts the PTY.
