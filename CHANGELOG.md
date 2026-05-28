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
  single-instance lock in `agentmux-app`.
- First TDD targets:
  - `agentmux-core::workspace` — pane tree + tabs with unit tests
    (open / split / focus).
  - `agentmux-claude::path` — `encode_project_path`, byte-faithful port of
    the JS reference, with unit tests.
