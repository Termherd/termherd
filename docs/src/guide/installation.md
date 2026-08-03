# Installation

## Requirements

A shell — and, to launch **Claude** sessions, the **Claude Code CLI 1.0.61 or
newer** on your `PATH`.

That floor is the CLI's `--settings` flag, which TermHerd puts on every Claude
launch. It re-enables the CLI's terminal title *for that session only*, and the
title is where a Claude session's activity status comes from: without it, a
`CLAUDE_CODE_DISABLE_TERMINAL_TITLE` anywhere in your own settings would leave
every session reading `starting` forever. An older CLI rejects the flag and the
launch fails. TermHerd's other flag, `--mcp-config` (the
[live bridge](../mcp/live-bridge.md)), has been available since 0.2.75.

A **plain shell** needs nothing installed — see
[Status and attention](../workspace/status.md) for what it takes to give one an
accurate status.

## Desktop installers

Each tagged release publishes installers on the
[Releases](https://github.com/Termherd/termherd/releases) page.

| Platform | File | Notes |
| --- | --- | --- |
| macOS | `TermHerd_<version>_<arch>.dmg` | Open it, drag **TermHerd** into Applications. |
| Windows | `*-setup.exe` (NSIS) | |
| Linux | `.deb` or `.AppImage` | `sudo apt install ./termherd_<version>_amd64.deb`, or `chmod +x` the AppImage. |

**Builds are not signed yet.** On macOS, first launch needs a right-click →
**Open**, or clearing the quarantine flag:

```bash
xattr -dr com.apple.quarantine /Applications/TermHerd.app
```

On Windows, SmartScreen may warn — choose **More info → Run anyway**.

## Bare command-line binary

The same releases carry one-line installers that drop `termherd` into your
Cargo bin directory:

```bash
# macOS / Linux
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/Termherd/termherd/releases/latest/download/termherd-installer.sh | sh
```

```powershell
# Windows
powershell -c "irm https://github.com/Termherd/termherd/releases/latest/download/termherd-installer.ps1 | iex"
```

## Verifying a Linux download

Linux release binaries carry a sigstore **keyless** build-provenance
attestation — no signing key; the signer is the release workflow itself, via
GitHub OIDC, logged in the public Rekor transparency log.

```bash
gh attestation verify termherd-x86_64-unknown-linux-gnu.tar.xz \
  --repo Termherd/termherd
```

A passing check proves both integrity and that the artifact was built by this
repository's CI. A `SHA256SUMS` file is attached to each release as well.

## From source

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.95.0, edition 2024);
`rustup` picks it up automatically.

```bash
git clone https://github.com/Termherd/termherd
cd termherd
cargo run -p termherd-app
```

## Where TermHerd writes

Never under `~/.claude`. Everything it owns lives in `~/.termherd`
(`%USERPROFILE%\.termherd` on Windows):

| Path | Holds |
| --- | --- |
| `~/.termherd/settings.json` | your settings — [reference](../reference/settings.md) |
| `~/.termherd/window.json` | window size and position |
| `~/.termherd/metadata.json` | stars, archives, custom session titles |
| `~/.termherd/captures/` | `capture-<ts>.json` / `.png` / `.gif` — see [Capture and record](../workspace/capture.md) |

TermHerd is **single-instance**: an advisory lock file under the system temp
directory. To run a second build alongside one that already holds the lock,
point it at a throwaway temp dir so its lock path differs:

```bash
TMPDIR=$(mktemp -d) cargo run -p termherd-app
```
