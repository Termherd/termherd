+++
id = "F-packaging-ci"
type = "feature"
area = ["packaging"]
status = "todo"
target = ["Must"]
+++

Signed macOS/Windows/Linux bundles behind a 3-OS CI gate.

Signed mac/win/linux builds + CI gate (3-OS matrix) — *bare-binary pipeline
(cargo-dist: curl|sh / PowerShell installers) plus the CI gate are in place;
desktop installers now build too — a `cargo-packager` config
(`[package.metadata.packager]` + an app icon set) and a `package.yml` workflow
produce macOS `.app`/`.dmg`, Windows `.msi`/`.exe` and Linux
`.deb`/`.AppImage`, attached to the release. macOS `.app`/`.dmg` verified
locally. Only "signed" remains — bundles are unsigned pending certificates
(OQ5). **Split by platform** (feature-torture 🧬). macOS: the Homebrew path
(#61) is **Parked** — Homebrew 5.1 removed `--no-quarantine` (all taps), so an
unsigned cask can't bypass Gatekeeper and casks failing it are unsupported
after 2026-09-01; v0.1.0 therefore ships macOS **unsigned** (`.dmg` + manual
`xattr`), and Developer ID notarization (#51, no free OSS path, now **Parked**)
is the sole fluent macOS path, deferred to GitHub traction / a sponsor
($99/yr). Linux ships **signed checksums** (#52, done) — a `sign-release.yml`
workflow attaches a `SHA256SUMS` over the Linux tarballs and a sigstore
*keyless* (GitHub OIDC, no stored key) build-provenance attestation; verify
with `gh attestation verify <artifact> --repo Termherd/termherd`. **Windows**
Authenticode via free **SignPath Foundation** (#62, now **Parked** — viable,
but not release-blocking)*
