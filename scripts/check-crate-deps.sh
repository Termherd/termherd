#!/usr/bin/env bash
# Assert the hexagonal crate dependency rule (internal deps point inward only)
# and toolchain version lockstep across config and workflow files.
set -euo pipefail

declare -A allowed=(
    [termherd-claude]=""
    [termherd-private-fs]=""
    [termherd-core]="termherd-claude"
    [termherd-scan]="termherd-core termherd-claude"
    [termherd-pty]="termherd-core termherd-claude termherd-private-fs"
    [termherd-mcp]="termherd-core termherd-claude"
    [termherd-app]="termherd-core termherd-claude termherd-scan termherd-pty termherd-mcp termherd-private-fs"
)

# Both jq calls feed process substitutions, where a failure does not trip
# `set -e`: without this check a missing jq reads as a workspace of 0 crates.
command -v jq >/dev/null || { echo "::error::jq is required (scripts/check-crate-deps.sh)." >&2; exit 1; }

metadata="$(cargo metadata --no-deps --format-version 1)"
cr=$'\r'

mapfile -t crates < <(jq -r '.packages[].name' <<<"$metadata" | sort)
crates=("${crates[@]%"$cr"}")
# The workspace always has crates: an empty list means the probe broke.
if [[ ${#crates[@]} -eq 0 || -z "${crates[0]}" ]]; then
    echo "::error::no crate read from cargo metadata; the dependency check cannot run." >&2
    exit 1
fi
violations=0
for crate in "${crates[@]}"; do
    if [[ -z "${allowed[$crate]+set}" ]]; then
        echo "::error::crate '$crate' is not in the dependency-rule allow-list (scripts/check-crate-deps.sh)." >&2
        violations=$((violations + 1))
    fi
done

while IFS=$'\t' read -r crate dep; do
    crate="${crate%"$cr"}"
    dep="${dep%"$cr"}"
    case " ${allowed[$crate]:-} " in
        *" $dep "*) ;;
        *)
            echo "::error::dependency-rule violation: '$crate' depends on '$dep' (see scripts/check-crate-deps.sh)." >&2
            violations=$((violations + 1))
            ;;
    esac
done < <(jq -r '
    .packages[] as $p
    | $p.dependencies[]
    | select(.name | startswith("termherd-"))
    | [$p.name, .name] | @tsv
' <<<"$metadata" | sort -u)

# Toolchain lockstep (Q10): all pins must match rust-toolchain.toml.
extract() { sed -ne "s/$1.*/\1/p" "$2" | tr -d '\r'; }
tc="$(extract '^channel = "\([^"]*\)"' rust-toolchain.toml)"
for pair in \
    "Cargo.toml:$(extract '^rust-version = "\([^"]*\)"' Cargo.toml)" \
    ".github/workflows/ci.yml:$(extract '^  RUST_TOOLCHAIN: "\([^"]*\)"' .github/workflows/ci.yml)" \
    ".github/workflows/codeql.yml:$(extract '^ *toolchain: "\([^"]*\)"' .github/workflows/codeql.yml)"; do
    file="${pair%%:*}"
    val="${pair#*:}"
    if [[ "$val" != "$tc" && "$tc" != "${val}."* ]]; then
        echo "::error::toolchain mismatch: rust-toolchain.toml has '$tc' but $file has '$val'." >&2
        violations=$((violations + 1))
    fi
done

if [[ "$violations" -ne 0 ]]; then
    echo "FAIL: $violations dependency-rule / toolchain lockstep violation(s)." >&2
    exit 1
fi

echo "OK: all ${#crates[@]} crates respect the hexagonal dependency rule and toolchain '$tc' is in lockstep."
