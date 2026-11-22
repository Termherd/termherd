# Product

## Register

product

## Users

A power Claude Code user who runs many sessions across many projects at once.
They live in the keyboard, keep termherd open all day beside their editor, and
switch between sessions dozens of times an hour. The job on every screen is
the same: see which session needs them, get to it, and get back out
(`docs/PRD.md` §3).

## Product Purpose

A native terminal workspace for Claude Code sessions: browse every session
Claude has written, launch, arrange in tabs and splits, monitor, and search,
on macOS, Windows and Linux alike. It is not an IDE: Claude keeps editing
through the user's own editor. Success is a user who never wonders which
session is waiting, and who finds any past session faster than they could
recall its name.

## Brand Personality

Quiet, precise, native. The chrome stays out of the terminal's way and speaks
only to report state. Every element earns its pixels, and the app feels like
a well-made desktop tool on each OS, never a web page in a window.

## Anti-references

- An Electron dashboard: SaaS cards, generous padding, a web app in a frame,
  like the Electron app termherd replaces.
- IDE clutter: VS Code-style panel stacks, toolbars and badges everywhere.
- The neon hacker terminal: green on black, glows, a costume of a terminal.
- Pure tmux, or too much TUI, as in herdr and Zellij: everything a memorised
  key, nothing to see or click.

## Design Principles

1. **State before chrome.** What a session is doing (busy, idle, waiting on
   the user) is the most important pixel on screen; everything else recedes.
2. **The terminal owns the pixels.** Chrome is dense and thin, and gives the
   grid every row it can.
3. **Keyboard first, never keyboard only.** Every action has a key, and every
   key has a visible place to discover it.
4. **The user's theme, faithfully.** Colour comes from the theme the user
   chose; termherd adds only the marks it needs and proves they read.
5. **Native on every OS.** One product on three platforms, each feeling at
   home, with no platform a second-class port.

## Market position

From a bench of comparable tools (2025-2026), read from their docs and
reviews, not from screenshots.

Codes to adopt:

| Ref | Code | Seen in |
| --- | --- | --- |
| A1 | Spatial attention signal: a ring on the pane, a lit tab, a key to jump to the next waiting session | cmux |
| A2 | Sidebar filterable and groupable by status | Claude Code desktop (2026) |
| A3 | A second context line per session: branch, Claude's last line | cmux |
| A4 | The system font and controls in the chrome | Ghostty, cmux |

Lines that set termherd apart:

| Ref | Difference | Against |
| --- | --- | --- |
| D1 | Native on all three OS, at parity | cmux, Conductor, Superset: macOS only |
| D2 | The history is the product: browse, search and resume any past session | Orchestrators that see only the agents they launched |
| D3 | The user's theme, with proven contrast | A house palette |
| D4 | No review-and-merge flow, no kanban | Conductor, Nimbalyst |

Avoid: Warp-style output blocks and per-agent worktrees, which pull toward
the IDE.

## Accessibility & Inclusion

WCAG 2.2 AA, as the code enforces today: 3:1 for marks and 4.5:1 for text on
every built-in theme, tested. Keyboard-first by design.
