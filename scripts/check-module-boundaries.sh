#!/usr/bin/env bash
# Architecture fitness function, one level down from the crate rule: assert the
# *intra-crate* module seams the split-up refactor established, so the walls
# built inside a crate hold by construction instead of by reviewer vigilance.
#
# `scripts/check-crate-deps.sh` guards the seams *between* crates. But the
# god-object kept reforming *inside* crates — `shell.rs` at 2000+ lines, one
# `App` with every method. Clusters A–E carved those into submodules with a
# deliberate dependency direction. Nothing stops a future edit from
# re-absorbing a submodule (a leaf importing its parent, a renderer reaching
# into the executor) and quietly rebuilding the blob. This gate freezes the
# direction.
#
# It is a **forbidden-import** checker: a table of (files, forbidden import
# pattern, reason). Each rule greps the named files for module-level `use`
# lines matching the pattern; any hit is a violation. Anchoring to `^use `
# (column 0) scans production imports only — test modules import their parent
# with an indented `use super::*` / `use crate::…`, which is scaffolding, not
# production coupling, so it is out of scope by construction.
#
# We chose a grep script over `cargo-modules` / `archtest-rs` (the tools the
# issue named to evaluate): the seams here are import-level and line-visible,
# the check needs no build and no pinned tool, and it mirrors the existing
# `check-crate-deps.sh` fitness-function pattern. `cargo-modules` would parse
# the whole crate to a graph — cost and a dependency out of proportion to a
# leaf-import rule. Add a rule below when a cluster establishes a new seam.
set -euo pipefail

# Each rule is three tab-separated fields:
#   files            — space-separated paths / globs the rule scans
#   forbidden_regex  — an ERE; a module-level `use` line matching it is illegal
#   reason           — printed on violation, in plain language (no bare codes)
#
# Anchor every regex with `^use ` so only top-level (production) imports match.
rules=$(cat <<'RULES'
crates/pty/src/input.rs	^use (crate|super)::(events|grid|status|kill|session|manager)\b	pty::input is a pure protocol leaf — it must not import a sibling module
crates/pty/src/grid.rs	^use (crate|super)::(events|input|status|kill|session|manager)\b	pty::grid is the rendering leaf — it must not import a sibling module
crates/pty/src/session.rs crates/pty/src/status.rs crates/pty/src/kill.rs crates/pty/src/events.rs	^use (crate::manager|super::manager)\b	nothing may import pty::manager — the PtyHost impl sits at the top; importing it back would form a cycle
crates/core/src/app/*.rs	^use (crate::app|super)::(session|tabs|sidebar|metadata|capture|record|settings|notify|events|effects)::	core::app submodules share state through the parent App (and its Sessions registry), never by reaching into a sibling submodule
crates/app/src/shell/view	^use (crate::shell::effects|(super::)+effects)\b	shell::view renders — it must not reach into shell::effects (the executor); effects execute, views only read
crates/app/src/shell/terminal	^use (crate::shell::effects|(super::)+effects)\b	shell::terminal renders — it must not reach into shell::effects (the executor)
RULES
)

violations=0
while IFS=$'\t' read -r files forbidden reason; do
    [[ -z "$files" ]] && continue
    # `files` may hold globs / directories; let the shell expand it, and scan
    # every .rs under any directory entry. Missing paths are a rule that has
    # drifted from the tree — surface it rather than silently passing.
    for path in $files; do
        if [[ -d "$path" ]]; then
            mapfile -t targets < <(find "$path" -name '*.rs')
        elif [[ -e "$path" ]]; then
            targets=("$path")
        else
            echo "::error::module-boundary rule references '$path', which does not exist (scripts/check-module-boundaries.sh). Update the rule." >&2
            violations=$((violations + 1))
            continue
        fi
        # An empty match (a directory rule over a dir with no .rs) leaves an
        # empty array; guard it so `set -u` doesn't abort on the expansion.
        [[ ${#targets[@]} -gt 0 ]] || continue
        for target in "${targets[@]}"; do
            while IFS= read -r hit; do
                [[ -z "$hit" ]] && continue
                echo "::error::module-boundary violation in ${target}: ${hit#*:} — ${reason}" >&2
                violations=$((violations + 1))
            done < <(grep -nE "$forbidden" "$target" || true)
        done
    done
done <<<"$rules"

if [[ "$violations" -ne 0 ]]; then
    echo "FAIL: $violations module-boundary violation(s)." >&2
    exit 1
fi

echo "OK: all intra-crate module seams respected."
