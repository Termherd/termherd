# TermHerd task runner. `just` lists recipes; `just <name>` runs one.
# Commands mirror AGENTS.md and .github/workflows/package.yml so local and CI
# builds stay in step.

# Show the recipe list when run with no arguments.
default:
    @just --list

# Run the app from source (debug).
run:
    cargo run -p termherd-app

# The single-instance guard keys off a lock file at `$TMPDIR/dev.termherd.lock`;
# a private TMPDIR gives this build its own lock, so it neither sees nor is
# blocked by an installed TermHerd.app's lock.
[doc("Run a dev copy alongside an installed TermHerd.app (own lock via private TMPDIR)")]
run-isolated:
    TMPDIR="$(mktemp -d)" cargo run -p termherd-app

# Assert the hexagonal crate dependency rule (deps point inward only). Mirrors
# the `deps` CI job; run before pushing a new crate or cross-crate dependency.
check-deps:
    ./scripts/check-crate-deps.sh

# Assert the intra-crate architecture rules (module boundaries + OS-cfg
# containment), then print the report-only file-length signal. Mirrors the
# `intra-arch` CI job; run before pushing a new module or an OS-conditional cfg.
check-arch:
    ./scripts/check-module-boundaries.sh
    ./scripts/check-os-cfg-containment.sh
    ./scripts/report-file-length.sh

# Report board/issue drift: an open issue absent from the board, one missing a
# field its issue type owes, or an item whose Status and Horizon disagree about
# having shipped. Run before a planning pass. Exits 0 clean / 1 drift found /
# 2 a dimension could not be fetched — a missing `project` scope reads as
# "unchecked", never as clean, which is also why this is not a CI gate: the
# default GITHUB_TOKEN lacks that scope. Stdlib-only, so plain python3.
[doc("Report board/issue drift (unclassified, or Status vs Horizon)")]
board-check:
    python3 scripts/board_check.py --check

[doc("Offline guards for the board check (no `gh`, no network)")]
board-check-selftest:
    python3 scripts/board_check.py --selftest

# Recompile ROADMAP.md from `.roadmap/`. ROADMAP.md is an artifact — edit the
# feature files, never it. Run this and commit both, or the `roadmap` CI job
# fails on the diff. CI pins the version, so match it locally:
#   cargo install roadmark --version 0.7.0 --locked
[doc("Recompile ROADMAP.md from .roadmap/, then validate the source")]
roadmap:
    roadmark generate -o ROADMAP.md
    roadmark validate

# Read-only: schema, duplicate ids, dead cross-references, and whether the
# committed ROADMAP.md still matches its source. Warnings (an empty body, prose
# naming something that isn't a feature) print without failing.
[doc("Check the roadmap source without rewriting ROADMAP.md")]
roadmap-check:
    roadmark validate

# Build the shipping binary (host target) — the input the packager bundles.
build-release:
    cargo build --release -p termherd-app

# Build the desktop bundle. Formats are pinned per OS to match
# .github/workflows/package.yml; auto-detection isn't safe (Windows would also
# try WiX/MSI, which rejects the `-prerelease.N` version suffix). cargo-packager
# only bundles an already-built binary, hence the `build-release` dep.

[doc("Build the desktop bundle (formats pinned per OS, matching CI)")]
[macos]
package: build-release
    cargo packager -p termherd-app --release --formats app,dmg

[doc("Build the desktop bundle (formats pinned per OS, matching CI)")]
[linux]
package: build-release
    # APPIMAGE_EXTRACT_AND_RUN lets AppImage tooling run without FUSE.
    APPIMAGE_EXTRACT_AND_RUN=1 cargo packager -p termherd-app --release --formats deb,appimage

[doc("Build the desktop bundle (formats pinned per OS, matching CI)")]
[windows]
package: build-release
    # NSIS only — WiX/MSI rejects the non-numeric `-prerelease.N` suffix.
    cargo packager -p termherd-app --release --formats nsis
