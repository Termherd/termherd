# Running and observing a build

How to run a build, observe it, and run one beside the user's own
termherd. Moved out of `AGENTS.md`; the `run-termherd` skill points here.

Some behaviour is GUI/OS-level and **cannot be exercised by a headless test**
— the macOS Cmd+Q quit-confirm flow, window placement, the PTY canvas. Verify
those by running the app and reading its `tracing` output:

```bash
# `tracing` is the only observation channel — there is no `println!`. Raise the
# level with RUST_LOG (default is `info,…`, see `DEFAULT_FILTER` in main.rs).
# It writes to STDERR, so redirect it when you pipe: `… 2>&1 | grep …`.
RUST_LOG=info cargo run -p termherd-app

# Add log lines at the seam you're verifying (info!/warn!, never println!), run,
# and grep the output for them — e.g. the quit path logs `request_quit`'s branch
# and the macOS menu repoint.
```

The app is **single-instance**, and its lock follows `std::env::temp_dir()` on
every OS (an flock there on macOS, a name hashed from it elsewhere). To run a
build *alongside* one that already holds the lock — common, since a dev/agent
session often runs *inside* a release `TermHerd.app` you can't quit — point the
new process at a throwaway temp dir so its lock path differs:

```bash
TMPDIR=$(mktemp -d) RUST_LOG=info cargo run -p termherd-app   # second instance
```

`temp_dir()` honours `$TMPDIR`, so both run. On Windows it reads `TMP` (then
`TEMP`), so set that instead. A missing temp dir is created at startup, with a
`temp dir was missing; created it` warning; one that cannot be created is
named in a warning, and the lock and the sessions' private files then fail
(#441: a Windows build pointed at a missing one used to hang with no log
line, and on macOS the lock went off unannounced). Launch detached when you
need to keep interacting with the original window (e.g. to compare quit
behaviour).

**A second instance shares `~/.termherd` with the first** unless told
otherwise. The lock moved, the settings did not: a theme picked in the test
build is persisted to the same file, and the settings watch repaints the
user's own app with it. The tab strip is restored at launch too, so a second
instance on the user's `layout.json` reopens their tabs, each Claude pane
running `claude --resume` on a conversation the first instance still holds.
Point the data dir at a throwaway directory:

```bash
TERMHERD_DATA_DIR=$(mktemp -d) TMPDIR=$(mktemp -d) cargo run -p termherd-app
```

On Windows, from Git Bash: `TMP`, not `TMPDIR`, and Windows paths — with
`TMPDIR` alone the build takes the user's lock and exits with code 2.

```bash
TERMHERD_DATA_DIR=$(cygpath -w "$(mktemp -d)") TMP=$(cygpath -w "$(mktemp -d)") cargo run -p termherd-app
```

`TERMHERD_DATA_DIR` moves `~/.termherd` and nothing else (`termherd-mcp`
reads it as well). Isolating with `USERPROFILE` or `HOME`, as this page used
to say, also hid `~/.claude` — an empty sidebar reporting
`projects directory not readable` — and handed the fake home to every shell,
so the build looked like no user's (#577). `CLAUDE_CONFIG_DIR`, read with
Claude Code's own precedence, points the build at another Claude directory.

## `just dev-instance`: the default second instance

`just dev-instance` does all of the above and points `CLAUDE_CONFIG_DIR` at
a fixture generated for the launch: three invented repositories (one session
in a worktree), nine sessions with AI titles, renames, `/color`s and
first-prompt fallbacks, each dated *relative to now*. It prints its root on
stderr as `dev instance: …`; the data dir is `<root>/data`.

The fixture rather than the user's `~/.claude`, always (#578):

- a screenshot of the real sidebar publishes the user's project names;
- the real directory is written to by every live Claude, so the build rescans
  every second or two and never shows the same thing twice;
- conformity to one developer's machine proves nothing about defaults, which
  unit tests and a pristine environment cover instead.

Its source is the `SESSIONS` table in
`crates/scan/examples/dev_fixture/fixture.rs`: edit it when a check needs
other sessions. The sidebar dates a session by its transcript's mtime, so the
generator sets each one; a frozen tree in git would age in every screenshot
and lose its mtimes on checkout. `crates/scan/tests/dev_fixture.rs` reads the
generated tree through `FsScanner` and spells out the rows it must produce.

## Traps a second instance still meets

**A build from before the lock followed the temp dir on Windows and Linux does
not see a newer one, nor the reverse**: the default lock name changed with it.
Quit an older release before testing a newer build against it. A pull
request branched before #413 is such a build: it would not start beside the
user's release while testing #432. Build it merged instead —
`git merge --no-commit origin/main`, build, `git merge --abort` — which is
also the tree that will land.

**A build an agent launches used to differ from a build a user launches.** The
agent's shell carries its own Claude session's `CLAUDE*` variables
(`CLAUDECODE`, `CLAUDE_CODE_CHILD_SESSION`, the session id and messaging
socket), and termherd passed its whole environment on to every PTY. A Claude
started in that build then wrote no `~/.claude/sessions/<pid>.json`, so nothing
it hosted could be named. termherd now strips the session-scoped ones before a
spawn (`INHERITED_SESSION_VARS` in `crates/pty/src/launch.rs`, #356). When a
new Claude Code release exports another variable naming its session, a build
an agent launches is where it shows first: compare `env | grep -i claude` in
an agent shell against that list.

**`cargo test` used to need `env -u ZDOTDIR` when run from inside a termherd
shell — it no longer does.** That shell exports
`ZDOTDIR=$TMPDIR/termherd-shell-<id>`, which a nested termherd read as the
user's own home: with the session ids restarting at 1 each run, the `.zshenv`
it then generated sourced *itself* (`job table full or recursion limit
exceeded`) and the shell never reached a prompt. It surfaced as
`typing_exit_into_a_real_shell_closes_the_tab_end_to_end` timing out and read
as a regression in whatever you had just changed. Fixed in #244: a directory
named `termherd-shell-*` is never replayed from, and the zsh recipe exports
`TERMHERD_ORIG_ZDOTDIR` so the real home crosses any nesting depth.

To reproduce that class of bug deliberately, poison the variable on the command
line — a test cannot do it for itself, since `std::env::set_var` is `unsafe` in
edition 2024 and the workspace denies `unsafe_code`:

```bash
env ZDOTDIR="$TMPDIR/termherd-shell-1" cargo test -p termherd-app \
  typing_exit_into_a_real_shell
```

## Driving an isolated build from outside

A dev build started beside the user's own termherd can be driven over its live
bridge by anything that speaks HTTP — no Claude session needed. Give it a
throwaway data dir and temp dir (so it neither takes the user's lock nor
touches their settings), turn `mcp.attach` on in that data dir, and read the
endpoint it publishes:

```bash
cargo build -p termherd-app
data=$(mktemp -d)
tmp=$(mktemp -d)
echo '{ "mcp": { "attach": true } }' > "$data/settings.json"
TERMHERD_DATA_DIR="$data" TMPDIR="$tmp" target/debug/termherd \
  > "$tmp/termherd.log" 2>&1 &

# Published once the server is bound: { "url": …, "token": … }, mode 0600.
until [ -s "$data/bridge.json" ]; do sleep 0.2; done
cat "$data/bridge.json"
```

`TERMHERD_MCP_ATTACH=1` on the launch line does the same as the settings file
for one launch, unless the file says `"never"`. With a `CARGO_TARGET_DIR`, the
binary is under that directory instead of `target/`.

The bridge speaks MCP's streamable HTTP in stateless mode: no
`mcp-session-id` to carry, and each POST answers with one JSON body. Every
call needs the bearer token and both `Accept` types; a wrong token gets `401`.

```bash
url=$(jq -r .url "$data/bridge.json")
token=$(jq -r .token "$data/bridge.json")
hdr=(-H "Authorization: Bearer $token" -H 'Content-Type: application/json'
     -H 'Accept: application/json, text/event-stream')

curl -s "${hdr[@]}" "$url" -d '{"jsonrpc":"2.0","id":1,"method":"initialize",
  "params":{"protocolVersion":"2025-06-18","capabilities":{},
  "clientInfo":{"name":"curl","version":"0"}}}'
curl -s "${hdr[@]}" "$url" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized"}'   # 202
curl -s "${hdr[@]}" "$url" -d '{"jsonrpc":"2.0","id":2,"method":"tools/call",
  "params":{"name":"snapshot","arguments":{}}}'
```

`target/debug/termherd mcp-config` (with the same `TERMHERD_DATA_DIR`) prints the
`mcpServers` snippet for that endpoint instead, after proving it answers — the
form to hand a Claude session started outside termherd. A file left by a
killed instance reads as stale there; the next launch over the same data
dir clears it.

## Capturing state for the AI dev loop (#108)

Press **⌘⇧S** (macOS) / **Ctrl+Shift+S** (rebindable as `capture`) to dump the
running app's state for an AI assistant to read — rung 0+1 of `F-capture`. Each
press writes a timestamped pair to `~/.termherd/captures/`:

- `capture-<ts>.json` — a diffable state dump of the whole workspace: focus,
  resolved config, the sidebar, every tab with its panes (each pane's stable
  handle, kind, cwd, status, the pid, peer name and session id of the
  Claude running in it, null when none is, and the colour its `/color` set,
  null when none), and the focused terminal's visible text. No vision
  needed.
- `capture-<ts>.png` — the real window pixels (iced `window::screenshot`), for
  render / colour / glyph bugs the text dump can't show.

The dump **is** the `WorkspaceSnapshot` the MCP `snapshot` tool reports, under a
fixed full filter (`SnapshotFilter::capture()`) — one model, two readers, so a
field never means one thing on disk and another on the wire.

`<ts>` is a UTC `YYYYMMDD-HHMMSS-mmm` stamp, so the **latest capture is the
highest-named pair** — an AI finds it by sorting the directory. Capture stays
pure in `core` (`Event::Capture(SnapshotInputs)` →
`Effect::Capture(WorkspaceSnapshot)`); all I/O — the clock, JSON/PNG encoding,
the files — lives in the `app` adapter (`crates/app/src/capture.rs`), which
shares its wire form with the MCP handler (`crates/app/src/snapshot_dto.rs`).

For motion (rung 2, #124), press **⌘⇧R** / **Ctrl+Shift+R** (rebindable as
`toggle-record`) to start a **GIF screencast**; press again to stop, or let it
auto-stop at the cap (default 8 fps / 30 s / 0.5× scale, set under a `record`
block in `settings.json` — #127). It writes `capture-<ts>.gif` to the same dir.
Same hexagonal split: `core` owns the
idle→recording state machine (frames are the time proxy — no clock), and the
`gif` encoder runs on a dedicated thread in `app` (`crates/app/src/record.rs`)
so the UI — and the recording — stay smooth.
