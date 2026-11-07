# Running and observing a build

How to run a build, observe it, and run one beside the user's own
termherd. Moved out of `AGENTS.md`; the `run-termherd` skill points here.

Some behaviour is GUI/OS-level and **cannot be exercised by a headless test**
— the macOS Cmd+Q quit-confirm flow, window placement, the PTY canvas. Verify
those by running the app and reading its `tracing` output:

```bash
# `tracing` is the only observation channel — there is no `println!`. Raise the
# level with RUST_LOG (default is `info,…`, see `DEFAULT_FILTER` in main.rs).
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
`TEMP`), so set that instead — to a directory that **exists**, as `mktemp -d`
guarantees. Pointed at a missing one, a Windows build never shows its window,
stops answering, and logs nothing about why (#441). Launch detached when you
need to keep interacting with the original window (e.g. to compare quit
behaviour).

**A second instance shares `~/.termherd/settings.json` with the first.** The
lock moved, the settings did not: a theme picked in the test build is
persisted to the same file, and the settings watch repaints the user's own
app with it. Point the home at a throwaway directory too. `paths::home_dir`
reads `USERPROFILE` before `HOME`, and a worktree-isolated agent session
refused a command setting `HOME` (it injects git config), so use
`USERPROFILE`:

```bash
USERPROFILE=$(mktemp -d) TMPDIR=$(mktemp -d) cargo run -p termherd-app
```

The home matters twice over since the tab strip is restored at launch: a
second instance on the user's `~/.termherd` reopens their tabs from
`layout.json`, so each Claude pane runs `claude --resume` on a conversation
the first instance is still holding, and whichever quits last writes the
file.

The fake home hides `~/.claude` as well, so that build starts with an empty
sidebar and finds no session files: fine for a theme or layout check, wrong
for anything that reads Claude's data; copy `~/.claude/projects` into the fake
home when a check needs real sessions.

**A build from before the lock followed the temp dir on Windows and Linux does
not see a newer one, nor the reverse**: the default lock name changed with it.
Quit an older release before testing a newer build against it. A pull
request branched before #413 is such a build: it would not start beside the
user's release while testing #432. Build it merged instead —
`git merge --no-commit origin/main`, build, `git merge --abort` — which is
also the tree that will land.

**A build an agent launches is not a build a user launches.** The agent's shell
carries its own Claude session's `CLAUDE*` variables (`CLAUDECODE`,
`CLAUDE_CODE_CHILD_SESSION`, the session id and messaging socket), and termherd
passes its whole environment on to every PTY. A Claude started in that build
then wrote no `~/.claude/sessions/<pid>.json`, so nothing it hosted could be
named: the agent-name copy read as broken in manual testing and worked at once
from a plain terminal. Until termherd strips those variables (#356), verify
anything that reads Claude's session files from a build the user starts in an
ordinary terminal.

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
