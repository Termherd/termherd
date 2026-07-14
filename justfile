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
