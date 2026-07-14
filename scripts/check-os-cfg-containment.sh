#!/usr/bin/env bash
# Architecture fitness function: keep OS-conditional compilation quarantined in
# a small, named set of files, the same spirit as the workspace-wide
# `unsafe_code = "deny"` rule (docs/CI.md §7). Platform `#[cfg]` scattered
# through otherwise-portable code is how cross-platform bugs hide — a Windows-
# only branch nobody on macOS ever compiles. Confining it to a handful of
# audited homes means every OS fork is in a file a reviewer knows to scrutinise.
#
# The gate greps for OS-conditional `cfg` *attributes* — `#[cfg(…)]`,
# `#![cfg(…)]`, `#[cfg_attr(…)]` — mentioning a platform predicate: `target_os`,
# `target_family`, `target_arch`, or a bare `unix` / `windows`. Matching the
# attribute form (not a bare `cfg(`) means prose that merely names `cfg(...)` in
# a doc comment is not a false positive. Any match in a file outside the
# allow-list fails CI. Build scripts (`build.rs`) are out of scope: their `cfg`
# is the *host* platform, a different axis — the scan only reads `*/src/**`.
#
# The runtime-boolean form `cfg!(target_os = …)` is deliberately NOT flagged:
# it compiles *every* branch on *every* platform and only picks one at runtime,
# so no code is ever hidden from another OS's compiler / clippy / tests — the
# exact hazard this quarantine targets (a Windows-only branch nobody on macOS
# builds) simply cannot arise. `cfg!()` is portable by construction; only
# compile-time *elision* needs confining. So `core`'s keymap can pick the
# primary modifier with `cfg!(target_os = "macos")` without a quarantine.
#
# A module rule (check-module-boundaries.sh) can't see `cfg`, hence a dedicated
# grep. Add a file to the allow-list — with its reason — only when a new OS fork
# genuinely needs its own home; the point is to keep the list short.
set -euo pipefail

# Files that MAY contain OS-conditional cfg, each with why it is a sanctioned
# home. Keep this list minimal and justified.
declare -A allowed=(
    [crates/app/src/main.rs]="gates the macos module (mod macos) — the composition root wiring the AppKit glue in"
    [crates/app/src/instance.rs]="single-instance lock naming differs per OS (path vs mutex-name vs abstract socket)"
    [crates/app/src/window_geometry.rs]="Linux window placement needs a fallback the other platforms don't"
    [crates/app/src/shell/effects/os.rs]="the OS effect handoffs — the shell's one sanctioned per-OS dispatch site"
    [crates/app/src/shell/session_ops.rs]="macOS-only quit reroute on window Opened (repoints Cmd+Q through the shell)"
    [crates/pty/src/kill.rs]="the kill reconciliation quarantine — Unix reaps, Windows does not"
    [crates/pty/src/status.rs]="mcp config is written with Unix-only permissions (0600) via a cfg fork"
)

# Attribute-form OS-cfg: `#[cfg(`, `#![cfg(`, `#[cfg_attr(` on a line naming a
# platform predicate. `unix` / `windows` match only as a bare predicate token —
# bounded by `(`, `,` or space on both sides (`cfg(unix)`, `not(unix)`,
# `any(unix, …)`) — so `feature = "windows-…"` (the word after a quote) is NOT a
# false positive; `target_os = "windows"` is still flagged via `target_os`.
# `#[cfg(test)]` never matches (no platform token). The `cfg!(…)` macro form is
# intentionally excluded (see the header) — it compiles everywhere, hiding
# nothing.
cfg_re='#!?\[cfg[_a-z]*\('
os_re='(target_os|target_family|target_arch|[(,[:space:]](unix|windows)[),[:space:]])'

violations=0
while IFS= read -r file; do
    rel="${file#./}"
    matches="$(grep -nE "$cfg_re" "$file" | grep -E "$os_re" || true)"
    [[ -z "$matches" ]] && continue
    if [[ -n "${allowed[$rel]+set}" ]]; then
        continue
    fi
    while IFS= read -r hit; do
        [[ -z "$hit" ]] && continue
        echo "::error::OS-cfg outside its sanctioned home in ${rel}: ${hit#*:} — move OS-conditional code into an allow-listed file (scripts/check-os-cfg-containment.sh) or add the file with a reason." >&2
        violations=$((violations + 1))
    done <<<"$matches"
done < <(find crates -path '*/src/*' -name '*.rs')

# Guard against the list drifting from the tree: an allow-listed file that no
# longer contains any OS-cfg should be pruned so the list stays honest.
for rel in "${!allowed[@]}"; do
    if [[ ! -e "$rel" ]]; then
        echo "::warning::allow-listed file '$rel' no longer exists (scripts/check-os-cfg-containment.sh); prune it." >&2
    elif ! grep -nE "$cfg_re" "$rel" | grep -qE "$os_re"; then
        echo "::warning::allow-listed file '$rel' no longer contains OS-cfg (scripts/check-os-cfg-containment.sh); prune it." >&2
    fi
done

if [[ "$violations" -ne 0 ]]; then
    echo "FAIL: $violations OS-cfg containment violation(s)." >&2
    exit 1
fi

echo "OK: OS-conditional cfg is confined to its sanctioned homes."
