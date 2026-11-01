#!/usr/bin/env bash
# Benchmark helpers, sourced on the host and in the container (docs/BENCH.md).
#
# Adapter contract. <tool>.Dockerfile builds `thbench-<tool>`, writes
# /bench-ref and installs procps, smem, bc and jq. tool-<tool>.sh defines:
#   start_app, wait_ready (say what "ready" means), app_roots (root pids),
#   open_first (one focused shell pane), open_more N, run_in_pane CMD,
#   proof NAME (evidence under /out).

# ---------------------------------------------------------------- host side

# The only OS-specific code: describing the host.
host_info() {
  local os cpu mem
  case "$(uname -s)" in
    Darwin)
      os="macOS $(sw_vers -productVersion 2>/dev/null)"
      cpu=$(sysctl -n machdep.cpu.brand_string 2>/dev/null)
      mem=$(( $(sysctl -n hw.memsize) / 1048576 )) ;;
    MINGW*|MSYS*|CYGWIN*)
      os="Windows ($(uname -s))"
      cpu=$(awk -F': ' '/model name/{print $2; exit}' /proc/cpuinfo)
      mem=$(awk '/MemTotal/{print int($2/1024)}' /proc/meminfo) ;;
    *)
      os="$(uname -s) $(uname -r)"
      cpu=$(awk -F': ' '/model name/{print $2; exit}' /proc/cpuinfo)
      mem=$(awk '/MemTotal/{print int($2/1024)}' /proc/meminfo) ;;
  esac
  echo "# host_os=$os"
  echo "# host_cpu=$cpu ($(getconf _NPROCESSORS_ONLN 2>/dev/null) logical)"
  echo "# host_mem_mb=$mem"
  echo "# docker=$(docker version --format '{{.Server.Version}}' 2>/dev/null)"
}

# Adapter folders, one per line. A lone drive letter ("C" in "C:/a") is glued
# back onto the folder after it.
adapter_dirs() {
  echo "$BENCH/adapters"
  local part drive="" IFS=:
  for part in ${BENCH_ADAPTERS:-}; do
    if [ ${#part} -eq 1 ]; then drive="$part:"; continue; fi
    [ -n "$part" ] && echo "$drive$part"
    drive=""
  done
}

# The folder holding adapter <tool>.
adapter_dir() {
  local d
  while IFS= read -r d; do
    if [ -f "$d/$1.Dockerfile" ] && [ -f "$d/tool-$1.sh" ]; then echo "$d"; return 0; fi
  done < <(adapter_dirs)
  echo "no adapter named '$1' in: $(adapter_dirs | tr '\n' ' ')" >&2
  return 1
}

# Adapter names, in search order.
adapter_names() {
  local d f
  while IFS= read -r d; do
    for f in "$d"/*.Dockerfile; do [ -f "$f" ] && basename "$f" .Dockerfile; done
  done < <(adapter_dirs)
}

# ------------------------------------------------------------ container side
# Tree figures come from cgroup v2, as in `docker stats`; app figures from
# /proc and smem, without the shells.

CG=/sys/fs/cgroup
SHELLS='^(bash|sh|dash|zsh|seq|cat|sleep|touch|tmux.*)$'

now() { date +%s.%N; }
cg_mem() {
  local cur inact
  cur=$(cat $CG/memory.current)
  inact=$(awk '/^inactive_file /{print $2}' $CG/memory.stat)
  echo $((cur - inact))
}
cg_cpu() { awk '/^usage_usec/{print $2}' $CG/cpu.stat; }

# The given pids and all their descendants.
tree_of() {
  local queue=("$@") out=() p kids
  while [ ${#queue[@]} -gt 0 ]; do
    p=${queue[0]}; queue=("${queue[@]:1}")
    out+=("$p")
    kids=$(ps -o pid= --ppid "$p" 2>/dev/null)
    for k in $kids; do queue+=("$k"); done
  done
  echo "${out[@]}"
}
# Drop the shells and the commands they run.
app_only() {
  local p c
  for p in "$@"; do
    c=$(cat /proc/$p/comm 2>/dev/null) || continue
    [[ $c =~ $SHELLS ]] || echo -n "$p "
  done
}
rss_kb() {
  local p s=0 v
  for p in "$@"; do
    v=$(awk '/^VmRSS/{print $2}' /proc/$p/status 2>/dev/null); s=$((s + ${v:-0}))
  done
  echo $s
}
pss_kb() {
  [ $# -eq 0 ] && { echo 0; return; }
  local re; re=$(printf '^%s$|' "$@"); re=${re%|}
  smem -H -c "pid pss" 2>/dev/null | awk -v re="$re" '$1 ~ re {s+=$2} END{print s+0}'
}
ticks() {
  local p s=0 v
  for p in "$@"; do
    v=$(awk '{print $14+$15}' /proc/$p/stat 2>/dev/null); s=$((s + ${v:-0}))
  done
  echo $s
}

# CPU in % of one core, averaged over the window.
idle_window() {
  local secs=$1; shift
  local roots=("$@") hz; hz=$(getconf CLK_TCK)
  local all app c0 c1 t0 t1 a0 a1
  all=$(tree_of "${roots[@]}"); app=$(app_only $all)
  c0=$(cg_cpu); a0=$(ticks $app); t0=$(now)
  sleep "$secs"
  c1=$(cg_cpu); a1=$(ticks $app); t1=$(now)
  all=$(tree_of "${roots[@]}"); app=$(app_only $all)
  echo "cg_mem_mb=$(echo "scale=1; $(cg_mem)/1048576" | bc)" \
    "app_rss_mb=$(echo "scale=1; $(rss_kb $all)/1024" | bc)" \
    "app_only_rss_mb=$(echo "scale=1; $(rss_kb $app)/1024" | bc)" \
    "app_only_pss_mb=$(echo "scale=1; $(pss_kb $app)/1024" | bc)" \
    "cpu_tree_pct=$(echo "scale=2; ($c1-$c0)/10000/($t1-$t0)" | bc)" \
    "cpu_app_pct=$(echo "scale=2; ($a1-$a0)*100/$hz/($t1-$t0)" | bc)" \
    "nproc_app=$(echo $app | wc -w) nproc_tree=$(echo $all | wc -w)"
}

# Samples until FILE exists and the app stays under BENCH_SETTLE_PCT of one
# core for two 1 s windows: the shell can finish while the app still parses
# what it queued.
sample_until() {
  local flag=$1; shift
  local roots=("$@") hz; hz=$(getconf CLK_TCK)
  local limit=${BENCH_SETTLE_PCT:-50}
  local t0 peak_cg=0 peak_app=0 m a all app
  local w_c0 w_a0 w_t0 cpk=0 c ac t pct apct calm=0 settle=""
  t0=$(now); w_c0=$(cg_cpu); w_t0=$t0
  all=$(tree_of "${roots[@]}"); app=$(app_only $all); w_a0=$(ticks $app)
  while :; do
    all=$(tree_of "${roots[@]}"); app=$(app_only $all)
    m=$(cg_mem); a=$(rss_kb $app)
    [ "$m" -gt "$peak_cg" ] && peak_cg=$m
    [ "$a" -gt "$peak_app" ] && peak_app=$a
    t=$(now)
    if [ "$(echo "$t - $w_t0 >= 1" | bc)" = 1 ]; then
      c=$(cg_cpu); ac=$(ticks $app)
      pct=$(echo "scale=0; ($c-$w_c0)/10000/($t-$w_t0)" | bc)
      apct=$(echo "scale=0; ($ac-$w_a0)*100/$hz/($t-$w_t0)" | bc)
      [ "$pct" -gt "$cpk" ] && cpk=$pct
      if [ -e "$flag" ] && [ "$apct" -lt "$limit" ]; then
        [ $calm -eq 0 ] && settle=$w_t0   # the calm began with this window
        calm=$((calm + 1))
      else calm=0; settle=""; fi
      w_c0=$c; w_a0=$ac; w_t0=$t
      [ $calm -ge 2 ] && break
    fi
    sleep 0.25
    if [ "$(echo "$(now) - $t0 > 600" | bc)" = 1 ]; then echo "timeout=1"; break; fi
  done
  local done_t; done_t=$(stat -c %.9Y "$flag" 2>/dev/null || now)
  echo "duration_s=$(echo "scale=2; $done_t - $t0" | bc)" \
    "settle_s=$(echo "scale=2; ${settle:-$(now)} - $t0" | bc)" \
    "peak_cg_mem_mb=$(echo "scale=1; $peak_cg/1048576" | bc)" \
    "peak_app_only_rss_mb=$(echo "scale=1; $peak_app/1024" | bc)" \
    "peak_cpu_tree_pct=$cpk"
}

# The S4 burst, identical for every adapter.
make_bigfile() {
  [ -f /root/big.txt ] || yes 'the quick brown fox jumps over the lazy dog 0123456789' \
    | head -c 52428800 > /root/big.txt
}
S4_CMD='seq 1 500000; cat /root/big.txt; touch /tmp/s4.done'

# One transcript line in the shape Claude Code writes.
jsonl_line() { # type role text timestamp cwd session-id
  printf '{"parentUuid":null,"isSidechain":false,"type":"%s","message":{"role":"%s","content":"%s"},"uuid":"%s","timestamp":"%s","cwd":"%s","sessionId":"%s","version":"2.1.0"}\n' \
    "$1" "$2" "$3" "$(cat /proc/sys/kernel/random/uuid)" "$4" "$5" "$6"
}
# One session file, padded to KB kilobytes.
make_session() { # dir cwd title timestamp kb
  local sid f pad
  sid=$(cat /proc/sys/kernel/random/uuid); f="$1/$sid.jsonl"
  jsonl_line user user "$3" "$4" "$2" "$sid" > "$f"
  pad=$(jsonl_line assistant assistant "$(printf 'Neutral benchmark text. %.0s' $(seq 1 40))" "$4" "$2" "$sid")
  [ "$5" -gt 0 ] && yes "$pad" | head -c "$(( $5 * 1024 ))" >> "$f"
  echo >> "$f"
}
# The project adapters click: newest session, so first in the sidebar.
make_work_project() {
  local d=/root/.claude/projects/-root-work
  mkdir -p /root/work "$d"
  make_session "$d" /root/work "Benchmark work project" "$(date -u +%Y-%m-%dT%H:%M:%S.000Z)" 0
}
# Neutral history, so no real session title reaches a capture.
make_history() {
  local n=${BENCH_HISTORY_SESSIONS:-13} p=${BENCH_HISTORY_PROJECTS:-3}
  local kb=$(( ${BENCH_HISTORY_MB:-40} * 1024 / ${BENCH_HISTORY_SESSIONS:-13} ))
  local i proj cwd d ts
  ts=$(date -u -d 'yesterday' +%Y-%m-%dT%H:%M:%S.000Z)
  for i in $(seq 1 "$n"); do
    proj=$(( (i - 1) % p + 1 )); cwd=/root/project-$proj
    d=/root/.claude/projects/-root-project-$proj
    mkdir -p "$cwd" "$d"
    make_session "$d" "$cwd" "Sample session $i" "$ts" "$kb"
  done
}
