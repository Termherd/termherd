#!/usr/bin/env bash
# Interleaved repetitions, one container at a time, then medians:
# run-all.sh <reps> <tool>...
set -u
BENCH=$(cd "$(dirname "$0")" && pwd)
REPS=$1; shift
for r in $(seq 1 "$REPS"); do
  for t in "$@"; do "$BENCH/run.sh" "$t" "$r" > /dev/null || exit $?; done
done
"$BENCH/summarize.sh"
