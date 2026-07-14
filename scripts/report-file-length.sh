#!/usr/bin/env bash
# Report-only signal (signal A from brainstorm/20260627-ci-quality-gates.md):
# the longest Rust source files. File length is a *proxy* — for weak domain
# boundaries, complexity, and merge-conflict risk — not a defect in itself, so
# this NEVER fails CI. It surfaces the same signal that flagged `shell.rs` at
# 2000+ lines before the split, so regrowth is visible in the job summary rather
# than discovered a year later. When a quality-report home exists (a dashboard /
# summary artifact), this feeds it; until then it prints to the CI step summary.
#
# Blocking length control already lives in clippy's `too_many_lines` (per
# function); this is the whole-file complement, kept report-only on purpose.
set -euo pipefail

# Files at or above this length are listed. A soft lens, not a limit.
THRESHOLD="${FILE_LENGTH_THRESHOLD:-400}"
TOP="${FILE_LENGTH_TOP:-15}"

# Longest first: line count + path, over all crate sources (tests included —
# an over-long test file is also a boundary smell).
mapfile -t ranked < <(
    find crates -path '*/src/*' -name '*.rs' -print0 \
        | xargs -0 wc -l \
        | awk '$2 != "total" { print }' \
        | sort -rn
)

emit() {
    printf '%s\n' "$@"
}

lines=()
lines+=("### File-length signal (report-only)")
lines+=("")
lines+=("Longest Rust sources — a proxy for complexity / merge-conflict risk, not a")
lines+=("gate. Top ${TOP}; files at or above ${THRESHOLD} lines flagged with ⚠️.")
lines+=("")
lines+=("| Lines | File |")
lines+=("| ---: | --- |")

count=0
flagged=0
for entry in "${ranked[@]}"; do
    n="$(awk '{print $1}' <<<"$entry")"
    path="$(awk '{$1=""; sub(/^ /,""); print}' <<<"$entry")"
    mark=""
    if [[ "$n" -ge "$THRESHOLD" ]]; then
        mark=" ⚠️"
        flagged=$((flagged + 1))
    fi
    count=$((count + 1))
    [[ "$count" -le "$TOP" ]] && lines+=("| ${n}${mark} | \`${path}\` |")
done

lines+=("")
lines+=("${flagged} file(s) at or above ${THRESHOLD} lines, of ${count} scanned.")

# Prefer the GitHub step summary; fall back to stdout for local runs.
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
    emit "${lines[@]}" >>"$GITHUB_STEP_SUMMARY"
fi
emit "${lines[@]}"
