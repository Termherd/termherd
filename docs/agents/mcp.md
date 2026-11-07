# Driving termherd over MCP

The design of the live MCP bridge and how it got there. `AGENTS.md` keeps
the rules an agent needs; the tool descriptions and the server's
instructions carry the calling rules. The `drive-termherd` skill points
here.

A Claude session **launched from termherd** gets an in-process MCP server wired
into its `mcpServers` at spawn (loopback, per-session token) — so it can read
and drive the workspace it runs in. This is the richer sibling of the capture
dump above: same `WorkspaceSnapshot` model, live instead of a file.

**Settled.** Twenty tools: `list_sessions` + `snapshot`
(perception), `open_session` / `split_pane` / `focus_pane` / `rename_tab` /
`close_pane` / `run_in_session` / `mouse_in_session` (action),
`claude_command` (arms the confirmation for a slash command typed into an
idle Claude — the one write path for edits Claude owns),
`wait_for_status` + `read_terminal` (synchronisation), `screenshot` (pixels),
`press_keys` + `run_action` (the app's own keyboard), `add_repo` +
`forget_repo` (membership — what the sidebar *contains*, as against what the
window draws), `prompt_in_session` (the loop below, composed), and
`list_options` + `set_option` (settings — served by both the live bridge and
the stdio server, over the one pure catalogue in `crates/mcp`, writing
`settings.json`, which the shell's settings watch then applies live; the one
pair that touches a file rather than `core`, so it needs no bridge round
trip). The loop
they exist to serve is **act → wait → observe**: `run_in_session` returns
immediately, so synchronise with `wait_for_status` and then `read_terminal`.
Wait for `busy` before `idle`: just after typing, Claude still shows the
status from before the prompt, and a wait on it answers at once (#435).
Do **not** poll `snapshot` in a loop — it races the transition you are
watching for, which is why the wait rung exists.

**That loop did not run until #236, and now does.** Every session used to sit
on `starting`, so `wait_for_status` only ever settled by timing out. Two
independent holes, both closed: a plain shell spoke none of the Claude OSC
dialect the status fold understood, and now runs with an injected OSC 133
shell-integration snippet — with the PTY's foreground process group standing in
where the snippet cannot apply, and nothing at all under ConPTY; and a
`CLAUDE_CODE_DISABLE_TERMINAL_TITLE` in the user's own `~/.claude/settings.json`
silenced the Claude channel outright, which a private `--settings` overlay on
the launch line now outranks. That overlay needs Claude Code 1.0.61; the
floor termherd states is higher, **2.0.73**, for the `--session-id` a fresh
launch carries — an older CLI rejects either flag and the launch fails.

The same stuck status also kept a close confirmation from arming for a *shell*
(`has_running_process` needs `Busy` or `Attention`); that follows from the fix
rather than needing one of its own, so a shell running a command now confirms
as a Claude session always did.

`screenshot` is the pixel companion to the text `snapshot`, for the render,
colour and glyph questions text cannot answer. Reach for it *last*: a
default-bound window is ~200 kB of PNG and a third more again as base64, where
a `snapshot` is a few hundred bytes. Two bounds keep that honest — `max_width`
(default 1200) and a total-pixel ceiling for tall windows a width alone never
reaches — and a window smaller than them is never upscaled. Shrinking averages
the covered pixels rather than picking the nearest one, so terminal glyphs stay
legible at the ~0.4× a retina window gets reduced by; that legibility is what
the bytes buy. Lower `max_width` when a coarse view will do. A headless run
has no window and says so as a tool-level error; the text reads keep working.

`press_keys` and `run_action` drive termherd's **own interface**, not a terminal
— raw keys *into* a session stay `run_in_session`'s job. They are the same
dispatch asked two ways: `press_keys` takes chords in `settings.json` syntax and
resolves them through the **live** keymap, so it tests the *binding* (including
the user's overrides); `run_action` takes the kebab-case action names (the
catalogue the stdio server publishes at `termherd://keys/schema`) and skips the
keymap, so it tests the *behaviour* and survives a rebind.

A chord is dispatched as a **synthesised key event** fed to `Shell::on_key` —
the whole ladder, not just `Keymap::lookup`. That is what makes `escape` and
`enter` reachable: they are *overlay* keys bound to no action, so an agent that
armed a close-confirmation would otherwise have no way to answer it and would
park the app until a human intervened. The corollary is that an open overlay
consumes an MCP press exactly as it consumes a keypress — reported per press,
naming the prompt in the way, so a caller learns why its chord did nothing.
`run_action` is gated on the same ladder, deliberately: neither tool may reach a
state the keyboard cannot.

Each press answers with what the ladder did — `ran` (with the action's name),
`inert` (nothing happened, with a `reason`), `overlay` (which prompt ate it),
`typed` (bound to nothing, so it reached the focused terminal), `unbound`
(nothing claimed it) — plus the resulting `focused_handle`. A malformed chord or
unknown action name rejects the **whole** call before anything applies: half an
applied sequence is worse than none, since the caller cannot tell how far it
got.

`inert` carries its `reason` because the two kinds of nothing call for opposite
responses: `no-surface` means the action is wired to nothing, so retrying is
pointless (`open-new-session` is the one), while `no-context` means a
precondition was absent — nothing focused to derive a repo from, no closed tab
to reopen, no tab to rename or open a menu on, nothing to scroll, nothing
selected to copy, no Claude agent name to copy — which the caller can go and
*create* before trying again (except the agent name on Windows, where none is
ever found). Every handler that can refuse that way says so at its own refusal
(they return `Option`), so no predicate here has to re-derive the list.

The line is whether the shell refused, **not** whether the effect was
interesting: `activate-tab-9` on a single-tab workspace reports `ran`, because
`core` applied the event and absorbed it. Collapsing the two would make the
distinction useless.

Sessions are addressed by a stable `handle` (the runtime `SessionId`), never
the Claude `resume_id`, which re-keys on a fork / plan-accept (Q6). Every call
is `tokio::timeout`-bounded in `BridgeHandle::call` (Q7).

Where it lives: tools in `crates/app/src/mcp/handler.rs`, transport in
`shell::bridge`, and the shell's answers in `shell::serve` — the one place an
external caller meets `core::App`. `core` has no MCP awareness at all; every
mutation goes through an existing `Event`. The keyboard rung adds
`shell::orchestrate::perform_presses` beside the action path, `input::event_of`
(the inverse of `chord_of`), and `routing::KeyboardOwner` / `KeyVerdict` — the
overlay ladder and its outcome named once, since three readers consult them.

**The pointer reaches a terminal, by cell** (#300). `mouse_in_session` places
one mouse event at a `(col, row)` of a session's visible screen — the pointer
half of what `run_in_session` does for text. The path mirrors the wheel's end
to end: `Event::TerminalPointer` → `Effect::TerminalPointer` →
`PtyHost::pointer` → the per-session terminal thread, which holds the live
scroll offset and the live mode. Bounds are checked in the shell against the
session's last `Screen`, and a cell outside them rejects the whole call, as a
malformed chord does.

**And the child gets it when it asked for it** (#155). Until then only wheel
notches were ever encoded to the PTY; a press over vim or Claude Code's
`/resume` was consumed as a local selection and the child never heard of it.
The rule is **one predicate, three readers**, all in `core::app::pointer`.
`MouseReporting` is the DECSET 1000 / 1002 / 1003 ladder the child negotiated;
`PointerEvent::route(Option<MouseReporting>)` answers `Forward` / `Select` /
`Nothing`, and under *any* reporting it never answers `Select` — the mouse is
the child's, so what its mode covers is forwarded and a motion it did not ask
for is dropped rather than drawn as a selection (a proptest pins that). The
three readers: the terminal thread routes on its **live** mode
(`pty::session::pointer_input`) and is the only one whose answer has effect;
the MCP shell routes on the last `Screen.mouse_reporting` to answer
`forwarded` / `selection` / `ignored`, the same lag class as its bounds
check; the canvas routes on the same field to hand the bare press / drag /
release / move to the child through `Message::TermPointer`, and keeps the
event for the terminal when a modifier is held — Shift is the xterm override
that lets a human select text out of a mouse-mode TUI, and the link modifier
opens links. The content-dependent gestures (double-click word, shift-extend,
link click) never left the canvas. *Where* a local gesture lands is still
`pointer_select` with the live offset; *whether* it is local at all is
`route`'s alone, and the bridge carries `route`'s own answer rather than a
renamed copy of it.

Three things the review pass after the merge added, each a rule worth
keeping. **A gesture's owner is decided at its press and kept to its
release** (`canvas::Held`): re-reading the modifiers on every event let a
Shift released mid-drag hand the terminal's selection to the child, and a
Shift pressed mid-drag steal the child's release. **A gesture the child holds
follows the pointer off the grid**, clamped to the border cell as xterm
reports it — otherwise a fast drag past a split ends with no release and the
TUI is left holding the button. And **motion is reported once per cell**, as
xterm does: iced hands the canvas one move per pixel, and under DECSET 1003
each one was a channel send, a PTY write, a redraw in the child *and* a full
`Screen` snapshot on the terminal thread, which the `Pointer` arm now skips
for anything that did not touch the grid (`session::PointerInput`), as the
`Foreground` poll already did.

Two structural points. The mode bits are read in **one** place, `pty::mode`
— a new leaf, because `input` (the encoder's gate) and `grid` (the `Screen`
snapshot) both need the reading and the module-boundary script forbids either
importing the other; that is the general move when two leaves need one fact.
And `wheel_bytes`' SGR/X10 body became the shared `mouse_report`, which
`mouse_bytes` extends to press, release, drag and motion — the encoder only
encodes; whether the mode covers the event is `route`'s question, asked by the
caller, so the gate is not restated inside it. What the report does **not**
carry is modifiers: the event lost its `modifiers` field in #311 as dead
weight, and it comes back as one type shared with `KeyMods` when a caller
needs it, not before.

**Still open.** Two features: `F-mcp-attach` (#267 — the bridge is reachable
only from a session termherd spawned, so the launcher itself cannot drive it)
and the pointer at termherd's own chrome (#301 — the sidebar, tabs and gutters
still have no mouse). Neither blocks the other.
One defect is open beside them: a long `claude_command` that Claude reads as
a paste (#439). The bridge's hardening is done: #284, then #293, which moved
the Host/Origin check from rmcp's defaults into termherd's own gate (403
before the token's 401). Three other defects are fixed. A `read_terminal`
that returned the screen from before the status it waited for (#436): the
terminal thread paces screens (one per 8 ms) but sent a status the moment it
decoded it, so the screen of the same bytes could follow. `report_status` now
publishes any held screen first, and a chunk's status goes out only after the
chunk is drawn. A status reading `idle` while Claude still
works (#427): `prompt_in_session` no longer settles on the status from before
its prompt (#435; a bare `wait_for_status` still needs `busy` first), and
Claude's idle title must stand 150 ms (`pty::status::IDLE_HOLD`) before a
busy session reads idle. And the copy chord overwriting a mouse-mode program's
own clipboard write with termherd's last copy (#316): the shell keeps no copy
cache any more, and asks the terminal whether it holds a selection at all.

**`F-mcp-agent-loop` shipped in #196 (`prompt_in_session`).** It composes
`run_in_session`, `wait_for_status`, and `read_terminal` into a single MCP round
trip, with bounded step-by-step execution and opt-in protection for nested Claude
sessions. `F-mcp-agent-loop` was a child of the #90 epic — with `screenshot`,
the keyboard tools, terminal pointer, and composed agent loop shipped, the live
bridge control surface is complete. It could only be built after #236: a
composed wait on a synchronisation that never fired would have settled by
timeout every time.

**Every overlay can now be left from the keyboard** (#237). An open sidebar
session-rename used to swallow every key including `escape`, parking the whole
control surface until a human cleared it with the mouse — the exact failure the
synthesised-key-event design exists to prevent, prevented for the confirmation
prompts and not for that one. `escape` now abandons the rename, matching the
blur it stands in for.

The interesting part is what closed it for good: the fix carries a sweep driven
off `KeyboardOwner::ALL` rather than a test per prompt, and that sweep found a
*second* offender nobody had reported — the doc editor, closable only by its own
button. A rung added without an exit fails there now. `escape` there goes
through the same close as the button, and since #248 a modified doc asks first
on both — and when another doc is clicked over it, and (since #385) on quit,
through the quit prompt itself. The confirmation is its own rung,
`DocDiscard`, whose `escape` hands the keyboard *back* to the editor, and the
sweep names that one hand-off rather than loosening "left" for every rung: an
empty ladder after one `escape` would
have forced the prompt to throw the edits away on the very key it exists to
make safe, while a general "the armed rung let go" would let any rung trade
one trap for another unseen.

Two rules the review of #248 paid for. **Arm a destructive prompt on a
deliberate gesture, never on an async answer**: arming the replace on the
read's reply raised it mid-typing, where the next Enter meant as a newline
would have answered it — so the click arms it, and a reply finding fresh edits
is dropped. **A modal scrim blocks the mouse, not the keyboard**: `opaque`
captures pointer events only, so a focused widget under a prompt still hears
the keys that answer it. The doc editor therefore takes input only while it
owns the keyboard (`doc_accepts_edits` is `keyboard_owner() == Some(Doc)`, the
rule the search box already followed) — so every prompt over it is covered, not
just its own — and the view and the edit handler both read that one predicate.

The sweep is the general form of the "test that claims to be exhaustive" rule
below: it is honest because `arm_overlay`'s `match` is compiler-checked, so a
new variant cannot slip past it. (**#236**, the sibling defect that left every
session on `starting`, is fixed — see the wait rung above.)

Both rename overlays now route `enter` to their existing commit messages,
alongside the widget's `on_submit` path for human input (#246).

**Two things named `mcp`.** `crates/mcp` is built: the stdio server
`termherd-mcp` and the pure option catalogue both servers share. The live
bridge runs the other way round from it and lives in `app` (`app::mcp`).
`docs/ARCHITECTURE.md` §15 also sketches an `mcp` adapter for
`F-mcp-ide-bridge` — termherd as an MCP *client* of Claude's IDE bridge — and
that one is unbuilt.
