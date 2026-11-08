#!/usr/bin/env bash
# In-container S1-S5 flow, common to all adapters (contract: top of lib.sh).
set -u
source /bench/lib.sh
source "/bench/tool-$1.sh"
mkdir -p /out

echo "# tool_ref=$(cat /bench-ref 2>/dev/null)"
echo "# limits=cpu.max:$(cat $CG/cpu.max) memory.max:$(cat $CG/memory.max)"
if [ -d /root/.claude/projects ]; then
  echo "# history=host copy (BENCH_REAL_HISTORY=1)"
else
  make_history
  echo "# history=generated sessions=${BENCH_HISTORY_SESSIONS:-13} projects=${BENCH_HISTORY_PROJECTS:-3} mb=${BENCH_HISTORY_MB:-40}"
fi
make_work_project
make_bigfile
rm -f /tmp/s4.done

t0=$(now); start_app; wait_ready; t1=$(now)
echo "S1 startup_s=$(echo "scale=2; $t1 - $t0" | bc)"
proof s1

open_first; proof s2-open
echo "S2 $(idle_window 30 $(app_roots))"

open_more 4; proof s3-open
echo "S3 $(idle_window 30 $(app_roots))"

run_in_pane "$S4_CMD"
echo "S4 $(sample_until /tmp/s4.done $(app_roots))"
proof s4

sleep 30
echo "S5 $(idle_window 30 $(app_roots))"
proof s5
