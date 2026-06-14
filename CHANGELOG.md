# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `F-keyboard-shortcuts` (M3): a configurable keymap (FR9). Every shortcut is
  now a `KeyChord -> Action` binding in `core::keymap` (pure, testable):
  human chords (`"ctrl+shift+c"`, order/case-insensitive) parse to a chord,
  platform-aware defaults bind copy/paste/close/search (⌘ on macOS, Ctrl
  elsewhere) plus `Ctrl+Tab` / `Ctrl+Shift+Tab` tab cycling, and the `keys`
  section of `settings.json` overrides any action (one chord or a list).
  Unknown actions and unparsable chords are logged and skipped. The shell
  builds a chord from each key event and runs the bound action, so the
  hard-coded clipboard chords are gone and keyboard tab switching, tab close
  and search focus now work; `split-*` / `focus-next/prev` actions are bound
  as those features land.
- `F-settings` (M3, thin): user settings (FR10) persisted to
  `~/.termherd/settings.json`. A **shell profile** (program + args) is injected
  into the `PtyManager` so each session launches the chosen shell instead of
  the platform default; a **GUI theme** (dark/light) dresses the iced chrome
  (sidebar, tab strip, buttons) while the terminal grid keeps its own colours.
  Every field defaults, so a missing or corrupt file still starts cleanly.
  Window bounds keep their own `window.json` (FR12); an in-app settings panel
  is the full version later, so for now the file is edited by hand.
- `F-builtin-terminal` (M2): clipboard copy/paste shortcuts (FR4). `Ctrl+V` /
  `Ctrl+Shift+V` paste the clipboard into the focused PTY (previously only
  copy-on-select existed, so there was no way to paste at all); the chord
  shadows the raw `^V` control byte, the Windows-terminal convention.
  `Ctrl+Shift+C` copies the current selection — plain `Ctrl+C` stays the
  interrupt signal, as in every terminal. On macOS the Cmd key drives copy
  (`Cmd+C`) and paste (`Cmd+V`) directly, leaving Ctrl free for the interrupt.
  Multi-line paste is handled
  correctly: newlines normalise to carriage returns, and when the application
  has enabled bracketed paste (DECSET 2004, which the `pty` `Screen` now
  reports) the text is wrapped in `ESC[200~`…`ESC[201~` so it lands as one
  block instead of submitting line by line.
- `F-session-tabs` (M3): open sessions are now navigable tabs, not just the
  last-launched terminal. A tab strip above the terminal switches between
  sessions; each chip carries its activity dot (the FR8 tab badge, folded to
  the most urgent status of the sessions it hosts) and a close button. Closing
  a tab kills its session's PTY — the first UI-driven `Effect::Kill` — and
  drops the session from the live registry and its cached screen. The tab tree
  edits are pure in `core` (`Workspace::activate`/`close_tab`, `Tab::sessions`,
  `App::tab_status`) behind `Event::ActivateTab`/`CloseTab`, exhaustively unit
  tested; the shell only renders the strip and forwards clicks. Drag-reorder
  (FR5) and keyboard tab-switching (`F-keyboard-shortcuts`) are not yet wired.
- `F-builtin-terminal` (M2): the embedded terminal is now a real terminal.
  The `pty` reader builds a `Screen` snapshot — the visible grid with per-cell
  RGB (resolved from the xterm-256 palette), the cursor and a scrolled flag —
  from `alacritty_terminal`'s renderable content. The shell draws it on an
  iced `canvas` (colour cells + cursor), routes raw keyboard to the focused
  PTY (control combos, named keys, cursor sequences, layout text; gated by a
  terminal/search focus model so the search box keeps its own typing),
  propagates window resize to the PTY's cell geometry, and resumes a Claude
  session with `claude --resume <id>` when a session row is clicked. Verified
  end-to-end on Windows: clicking a session resumed a live Claude run inside
  TermHerd, its OSC activity drove the badge, and keystrokes reached it.
- `F-builtin-terminal` (M2, completed): wheel scrollback and drag-to-select
  with copy-to-clipboard close out FR4. The `pty` adapter now runs a reader
  thread (blocking PTY reads) feeding a terminal thread that owns the grid
  and applies bytes / resize / scroll commands, so the viewport reacts to the
  wheel immediately instead of waiting on the next PTY output. Selection is
  tracked in the canvas, highlighted, and copied on release.
- `F-status-notifications` (M2, completed for the current surfaces): the
  `pty` reader thread decodes each raw chunk with `termherd_claude::osc`
  *before* the bytes reach `alacritty_terminal` (which would consume the
  sequences) and folds the markers into a per-session status, emitting
  `PtyEvent::Status` on change. Beyond busy/idle, an OSC 9 notification — a
  permission prompt or an explicit "needs your attention" ping — now maps to
  a distinct `Attention` status; it is sticky against a bare idle prompt (the
  user still has to act) and cleared only when work resumes. The shell feeds
  it to `core` (`Event::StatusChanged`) and surfaces it as a coloured badge on
  the focused terminal *and* as a per-session dot in the sidebar (`core` now
  records which Claude session each terminal resumed, so a browsed row shows
  its live activity). Tab badges arrive with tabs (M3); the bell is decoded
  but deliberately not treated as an activity status.
- `F-builtin-terminal` (M2, in progress): `termherd-pty` adapter — one
  `portable-pty` PTY + `alacritty_terminal` grid per session, owned by its
  own reader thread (actor-per-session, the structural fix for the
  `realSessionId` race, Q6). A `PtyResponder` replies to cursor-position
  queries (`ESC[6n`), without which ConPTY never starts the child on
  Windows. The headless core gained the terminal lifecycle —
  `Event::LaunchSession`/`TerminalInput`/`TerminalResized`/`PtyExited` and
  `Effect::Spawn`/`Write`/`Resize`/`Kill` over a monotonic `SessionId`
  registry — and the iced shell performs those effects against the
  `PtyHost` port: clicking a project opens a terminal, its live screen
  renders as monospace text with a line-input box. Verified end-to-end on
  Windows ConPTY (spawn → reply → write → grid → kill) and visually in the
  shell. Pending: raw key input, colours/cursor/selection, scrollback,
  widget-driven resize.
- Initial scaffold: Cargo workspace (`core` / `claude` / `app`), pinned
  toolchain (1.95.0), MIT license, README, deny config.
- CI: `fmt`, `clippy -D warnings`, `cargo test`, `cargo-deny`, markdownlint
  required on PR (Q2).
- `F-foundations` (M0): workspace skeleton, dependency rule, `tracing` init,
  single-instance lock in `termherd-app`.
- `F-app-shell` (M0): iced 0.14 window shell (OQ1 settled on iced) —
  placeholder view, window bounds persisted to `~/.termherd/window.json`
  on close and restored on launch (FR12); close requests intercepted so
  bounds always save. The menu is deferred to M3: iced has no native menu
  API, and the menu will mirror keymap actions (`F-keyboard-shortcuts`),
  so they land together.
- `F-search` (M1): in-memory search (FR3) — case-insensitive over titles,
  summaries, slugs and indexed text, titles-only toggle; pure
  `filter_projects` in `core` behind `Event::SearchChanged`, search box +
  checkbox in the sidebar.
- `F-session-browser` (M1, completed): debounced `notify` watch on
  `~/.claude/projects` (FR2) — bursts of fs events coalesce into one
  rescan; the sidebar live-updates while Claude CLI writes. Verified
  live: create/delete in the projects tree triggers a ~200 ms rescan.
- `F-session-browser` (M1, first slice): `termherd-scan` adapter — walks
  `~/.claude/projects` with upstream's exact derivation order (direct
  JSONL, then session subdirs and `subagents/`), worktree collapse with
  the fs existence check, underivable folders dropped like upstream but
  logged; `core::browser` — pure grouping (one group per real path, FR1;
  recency ordering) behind `Event::ScanCompleted`; the shell scans off
  the UI thread at startup (FR2, initial scan only) and renders the
  sidebar. Live `notify` updates and FTS search still to come in M1.
- `F-packaging-ci` (M0, unsigned): cargo-dist 0.32 release pipeline —
  tag-triggered GitHub workflow building mac (ARM + x64), Linux
  (x64 + ARM) and Windows artifacts with shell/PowerShell installers and
  checksums. Windows artifact verified locally (10.6 MB binary vs the
  Electron app's ~150 MB). Signing/notarization pending certificates
  (OQ5).
- First TDD targets:
  - `termherd-core::workspace` — pane tree + tabs with unit tests
    (open / split / focus).
  - `termherd-claude::path` — `encode_project_path`, byte-faithful port of
    the JS reference, with unit tests.
  - `termherd-claude::derive` — real-project-path recovery (`extract_cwd`
    from JSONL, worktree collapse), ported from `derive-project-path.js`;
    unit + property tests.
  - `termherd-claude::digest` — session digest (summary, title precedence
    per the #46 contract, message counts, FTS text), ported from
    `read-session-file.js`; deliberately skips corrupt lines instead of
    dropping the whole session (Q5); unit + property tests.
  - `termherd-claude::osc` — PTY status decoding (busy spinner / ✳ idle /
    OSC 9 notifications / alt-screen / bell), ported from the inline
    `main.js` parsing; unit + property tests.
  - Codec validated against a real `~/.claude/projects` tree: every derived
    `cwd` re-encodes to its folder name; all sessions digested.
- `docs/background/` — imported the four 2026-05-27 analysis docs that
  produced the restart decision (assessment, feature sizing, the Electron
  app's architecture and NFRs) plus an index README.
