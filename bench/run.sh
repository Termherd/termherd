#!/usr/bin/env bash
# One S1-S5 run of one adapter, in a capped container: run.sh <tool> <rep>
set -u
BENCH=$(cd "$(dirname "$0")" && pwd)
source "$BENCH/lib.sh"
T=$1; R=$2
ADIR=$(adapter_dir "$T") || exit 2
OUT=${BENCH_RESULTS:-$BENCH/results}/$T-rep$R
N=thbench-$T-run
mkdir -p "$OUT"

if [ -n "$(docker ps -q --filter name=bench)" ]; then
  echo "another benchmark container is running; two measures at once skew both" >&2
  exit 3
fi

# Stage the harness and the adapter for one docker cp.
STAGE=$(mktemp -d)
cp "$BENCH/lib.sh" "$BENCH/drive.sh" "$BENCH/tool-gui.inc.sh" "$ADIR/tool-$T.sh" "$STAGE/"

docker run -d --name "$N" --cpus "${BENCH_CPUS:-4}" --memory "${BENCH_MEM:-4g}" \
  --shm-size 1g "thbench-$T" sleep infinity >/dev/null || exit 4
docker cp "$STAGE/." "$N:/bench" >/dev/null
rm -rf "$STAGE"
if [ "${BENCH_REAL_HISTORY:-0}" = 1 ]; then
  docker exec "$N" sh -c 'mkdir -p /root/.claude'
  docker cp "$HOME/.claude/projects" "$N:/root/.claude/projects" >/dev/null
fi

{
  echo "# date_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  host_info
  echo "# adapter=$ADIR"
} > "$OUT/metrics.txt"

( while docker inspect -f '{{.State.Running}}' "$N" 2>/dev/null | grep -q true; do
    echo "$(date +%s) $(docker stats --no-stream --format '{{.MemUsage}} {{.CPUPerc}}' "$N")"
    sleep 2
  done ) > "$OUT/docker-stats.txt" 2>/dev/null &
SP=$!
docker exec \
  -e BENCH_HISTORY_SESSIONS -e BENCH_HISTORY_PROJECTS -e BENCH_HISTORY_MB \
  "$N" sh -c "bash /bench/drive.sh $T" 2>"$OUT/stderr.txt" | tee -a "$OUT/metrics.txt"
docker cp "$N:/out/." "$OUT/" >/dev/null 2>&1
docker rm -f "$N" >/dev/null
wait $SP 2>/dev/null
