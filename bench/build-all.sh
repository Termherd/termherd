#!/usr/bin/env bash
# Build adapter images (all when none named), sequentially and uncached so the
# timings are comparable: build-all.sh [tool...]
set -u
BENCH=$(cd "$(dirname "$0")" && pwd)
source "$BENCH/lib.sh"
RES=${BENCH_RESULTS:-$BENCH/results}
mkdir -p "$RES"
tools=("$@"); [ ${#tools[@]} -eq 0 ] && mapfile -t tools < <(adapter_names)
rc=0
for t in "${tools[@]}"; do
  d=$(adapter_dir "$t") || { rc=2; continue; }
  s=$(date +%s)
  # shellcheck disable=SC2086 # BENCH_BUILD_ARGS is a list of flags
  docker build --no-cache --progress=plain ${BENCH_BUILD_ARGS:-} \
    -f "$d/$t.Dockerfile" -t "thbench-$t" "$d" > "$RES/build-$t.log" 2>&1
  r=$?
  echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) $t rc=$r wall_s=$(( $(date +%s) - s ))" | tee -a "$RES/build-times.txt"
  [ $r -ne 0 ] && rc=1
done
exit $rc
