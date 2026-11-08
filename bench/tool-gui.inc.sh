# GUI adapter helpers. "Painted": Xvfb's root stays black until the window draws.
export DISPLAY=:99
start_x() {
  Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp >/tmp/xvfb.log 2>&1 &
  until xdotool getmouselocation >/dev/null 2>&1; do sleep 0.05; done
}
pixel() { import -window root -crop 1x1+$1+$2 txt:- 2>/dev/null | tail -1 | grep -o 'srgb([^)]*)'; }
wait_painted() { while [ "$(pixel "$1" "$2")" = "srgb(0,0,0)" ] || [ -z "$(pixel "$1" "$2")" ]; do sleep 0.02; done; }
focus_win() { local w; w=$(xdotool search --onlyvisible --name "$1" | head -1); xdotool windowfocus --sync "$w"; }
type_line() { xdotool type --delay 5 "$1"; xdotool key Return; }
proof() { import -window root "/out/$1.png"; }
