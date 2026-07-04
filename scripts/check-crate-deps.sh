#!/usr/bin/env bash
# Architecture fitness function: assert the hexagonal dependency rule at the
# crate seam. Internal (termherd-*) dependencies must point INWARD only.
#
#   claude  -> {}                 pure codec, leaf — depends on nothing internal
#   core    -> {claude}           domain — only the codec, no adapters
#   scan    -> {core, claude}     adapters — core + the codec, never each other
#   pty     -> {core, claude}
#   mcp     -> {core, claude}
#   app     -> {core, claude, scan, pty, mcp}   the shell — may wire anything
#
# AGENTS.md / docs/ARCHITECTURE.md declare this Ports & Adapters invariant
# ("adapters depend on core, never the reverse"). This makes it hold by
# construction instead of by reviewer vigilance.
#
# Reads `cargo metadata --no-deps` (workspace crates only) and checks every
# internal dependency — of any kind (normal, dev, build) — against the
# allow-list below. A new internal edge that isn't allow-listed fails CI; add
# it here deliberately, with the reason, when the architecture genuinely grows.
set -euo pipefail

# allowed[crate] = space-separated set of internal crates it MAY depend on.
declare -A allowed=(
    [termherd-claude]=""
    [termherd-core]="termherd-claude"
    [termherd-scan]="termherd-core termherd-claude"
    [termherd-pty]="termherd-core termherd-claude"
    [termherd-mcp]="termherd-core termherd-claude"
    [termherd-app]="termherd-core termherd-claude termherd-scan termherd-pty termherd-mcp"
)

metadata="$(cargo metadata --no-deps --format-version 1)"

# Windows jq emits CRLF; strip the CR everywhere or nothing matches the lists.
cr=$'\r'

# Every workspace crate must appear in the allow-list, else the map has drifted
# from the workspace (a crate was added/renamed without updating this rule).
mapfile -t crates < <(jq -r '.packages[].name' <<<"$metadata" | sort)
crates=("${crates[@]%"$cr"}")
violations=0
for crate in "${crates[@]}"; do
    if [[ -z "${allowed[$crate]+set}" ]]; then
        echo "::error::crate '$crate' is not in the dependency-rule allow-list (scripts/check-crate-deps.sh). Add it with its allowed internal deps." >&2
        violations=$((violations + 1))
    fi
done

# For each crate, every internal (termherd-*) dependency must be allow-listed.
while IFS=$'\t' read -r crate dep; do
    crate="${crate%"$cr"}"
    dep="${dep%"$cr"}"
    case " ${allowed[$crate]:-} " in
        *" $dep "*) ;; # allowed
        *)
            echo "::error::dependency-rule violation: '$crate' depends on '$dep', which the hexagonal rule forbids (see scripts/check-crate-deps.sh)." >&2
            violations=$((violations + 1))
            ;;
    esac
done < <(jq -r '
    .packages[] as $p
    | $p.dependencies[]
    | select(.name | startswith("termherd-"))
    | [$p.name, .name] | @tsv
' <<<"$metadata" | sort -u)

if [[ "$violations" -ne 0 ]]; then
    echo "FAIL: $violations dependency-rule violation(s)." >&2
    exit 1
fi

echo "OK: all ${#crates[@]} crates respect the hexagonal dependency rule."
