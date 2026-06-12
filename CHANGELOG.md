# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Initial scaffold: Cargo workspace (`core` / `claude` / `app`), pinned
  toolchain (1.95.0), MIT license, README, deny config.
- CI: `fmt`, `clippy -D warnings`, `cargo test`, `cargo-deny`, markdownlint
  required on PR (Q2).
- `F-foundations` (M0): workspace skeleton, dependency rule, `tracing` init,
  single-instance lock in `termherd-app`.
- `F-app-shell` (M0, partial): iced 0.14 window shell (OQ1 settled on
  iced) — placeholder view, window bounds persisted to
  `~/.termherd/window.json` on close and restored on launch (FR12); close
  requests intercepted so bounds always save. Menu still to come.
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
