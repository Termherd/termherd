#!/usr/bin/env bash
# Exercise scripts/check-crate-deps.sh against a broken probe: it must fail,
# never report OK on a crate list it could not read. Prints PASS/FAIL per case
# and exits non-zero on any failure. Run from the repository root.
set -u
script=./scripts/check-crate-deps.sh
tmp=$(mktemp -d "${TMPDIR:-/tmp}/t.XXXXXX")
fail=0

# A PATH holding only the tools the script needs, so one of them can be left
# out or replaced without touching the host.
shim() { # dir, tool...
  local dir=$1 tool
  shift
  mkdir -p "$dir"
  for tool in "$@"; do ln -s "$(command -v "$tool")" "$dir/$tool"; done
}
base=(bash env cargo rustc sort sed tr)

expect() { # name, want-exit, PATH, stderr pattern naming the guard that fired
  local got
  PATH=$3 "$script" >"$tmp/out" 2>"$tmp/err"
  got=$?
  # A failing case must not also print the OK line, and must fail on the
  # guard it targets, not on another one that happens to catch it.
  if [[ $got -eq $2 ]] && { [[ $2 -eq 0 ]] || { ! grep -q '^OK' "$tmp/out" && grep -q "$4" "$tmp/err"; }; }; then
    echo "PASS $1 ($got)"
  else
    echo "FAIL $1: want exit $2${4:+ from the guard saying '$4'}, got $got"
    cat "$tmp/out" "$tmp/err"
    fail=1
  fi
}

shim "$tmp/nojq" "${base[@]}"
expect "jq missing fails" 1 "$tmp/nojq" "jq is required"

shim "$tmp/emptyjq" "${base[@]}"
printf '#!/usr/bin/env bash\nwhile read -r _; do :; done\n' > "$tmp/emptyjq/jq"
chmod +x "$tmp/emptyjq/jq"
expect "empty crate list fails" 1 "$tmp/emptyjq" "no crate read"

shim "$tmp/real" "${base[@]}" jq
expect "real workspace passes" 0 "$tmp/real"

# A cargo that answers with a doctored workspace, so an edge can go unused
# without editing a Cargo.toml.
metadata=$(cargo metadata --no-deps --format-version 1)
shim "$tmp/unused" bash env rustc sort sed tr jq cat
jq '(.packages[] | select(.name == "termherd-mcp") | .dependencies) |= []' \
  <<<"$metadata" > "$tmp/unused/metadata.json"
printf '#!/usr/bin/env bash\ncat "%s"\n' "$tmp/unused/metadata.json" > "$tmp/unused/cargo"
chmod +x "$tmp/unused/cargo"
expect "an allowed edge no crate uses fails" 1 "$tmp/unused" "may depend on 'termherd-core' but does not"

# The crate list in AGENTS.md must name every workspace member, and nothing
# else, so a crate cannot be added or dropped without its line.
sed '/^- `crates\/private-fs`/d' AGENTS.md > "$tmp/agents-missing.md"
AGENTS_MD="$tmp/agents-missing.md" expect "a crate AGENTS.md omits fails" 1 "$tmp/real" "'termherd-private-fs' is not listed"
awk '/^- Unbuilt: /{print "- `crates/store` — gone."}1' AGENTS.md > "$tmp/agents-extra.md"
AGENTS_MD="$tmp/agents-extra.md" expect "a crate AGENTS.md invents fails" 1 "$tmp/real" "lists 'crates/store'"

rm -rf "$tmp"
exit $fail
