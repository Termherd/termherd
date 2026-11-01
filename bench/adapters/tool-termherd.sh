# Coordinates assume the default 1100x720 window centred on 1600x1000, with
# /root/work on the first sidebar row (make_work_project).
source /bench/tool-gui.inc.sh
start_x
start_app() { termherd >/out/termherd.log 2>&1 & }
wait_ready() { wait_painted 900 300; }
app_roots() { pgrep -x termherd | while read -r p; do [ "$(awk '{print $3}' /proc/$p/stat)" != Z ] && echo -n "$p "; done; }
n=0
open_first() {
  sleep 3   # let the sidebar scan finish so the project row is in place
  focus_win TermHerd; xdotool mousemove 506 291 click 1; sleep 1   # the row's shell button
  xdotool mousemove 900 500 click 1; n=1; sleep 0.5
}
# Off macOS, split-horizontal; the new pane takes focus.
open_more() { while [ $n -lt "$1" ]; do xdotool key alt+shift+d; n=$((n+1)); sleep 1; done; }
run_in_pane() { type_line "$1"; }
