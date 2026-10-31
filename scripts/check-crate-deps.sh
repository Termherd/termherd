#!/usr/bin/env bash
# Assert the hexagonal crate dependency rule (internal deps point inward only)
# and toolchain version lockstep across config and workflow files.
set -euo pipefail

declare -A allowed=(
    [termherd-claude]=""
    [termherd-core]="termherd-claude"
    [termherd-scan]="termherd-core termherd-claude"
    [termherd-pty]="termherd-core termherd-claude"
    [termherd-mcp]="termherd-core termherd-claude"
    [termherd-app]="termherd-core termherd-claude termherd-scan termherd-pty termherd-mcp"
)

metadata="$(cargo metadata --no-deps --format-version 1)"
cr=$'\r'

mapfile -t crates < <(jq -r '.packages[].name' <<<"$metadata" | sort)
crates=("${crates[@]%"$cr"}")
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
