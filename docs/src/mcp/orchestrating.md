# Orchestrating sessions

Recipes for one Claude session driving others through the
[live bridge](./live-bridge.md): opening workers, handing them work, waiting
for them and reading what they did. Every tool named here is a live-bridge
tool; a Claude session lists them as `mcp__termherd__<tool>`.

The driving session must be one TermHerd launched as a Claude tab, or a client
[attached from outside](./attach.md). A `claude` typed by hand into a shell
pane has no bridge.

## Find a session, and keep its handle

`list_sessions` answers one row per live session. Address every later call by
the row's **`handle`**. Never use `resume_id`: Claude Code re-keys a
conversation on a fork or a plan-accept, while the handle stays the same for
as long as the session lives.

Each row also says what the session runs (`kind` is `shell` or `claude`), its
working directory, its status, and, for a Claude, the `peer_name` other Claude
sessions address it by. Two Claude tabs in the same directory are told apart
by their handle and `peer_name`, not by their title.

## Open a worker without taking the user's keyboard

```json
{ "tool": "open_session",
  "args": { "project": "/path/to/worktree", "kind": "claude", "background": true } }
```

`background: true` appends the tab without activating it, so the next key the
user types still lands where they were typing. Read the new session from
`opened_handle`, not from `focused_handle`. A new Claude session starts on
`starting`: wait for `idle` before handing it work.

Close a finished worker the same way, with `close_pane` and
`background: true`.

## Hand work over as a brief file

Write the task into a file in the worker's own directory — say
`.personal/brief.md` in its worktree — and type one short line:

```json
{ "tool": "run_in_session",
  "args": { "session": "12", "text": "Read .personal/brief.md and carry it out.\n" } }
```

A file can be as long as the task needs, the user can read and amend it, and
it outlives the conversation that wrote it. A one-line prompt is typed
reliably, and the worker reads the whole brief with its own tools. A trailing
newline in `text` is sent as Enter and submits; earlier newlines stay line
breaks, so a multi-line prompt is still submitted once.

## Act, wait, observe

`run_in_session` returns as soon as the text is sent. Follow it with a wait,
then a read:

1. `run_in_session` — type the prompt.
2. `wait_for_status` with `statuses: ["busy"]` — just after typing, the
   session still shows the status it had *before*, so a wait for `idle`
   straight away answers at once with the old one.
3. `wait_for_status` with `statuses: ["idle", "attention"]`.
4. `read_terminal` — the last lines of the screen, as text.

`prompt_in_session` does all three in one round trip: it types, waits up to
2 s for the status to leave its old value, waits for the target statuses, and
returns the screen text. Prefer it for a prompt; keep the separate calls when
you need to act between the steps.

Never poll `snapshot` to wait: it races the very transition you are waiting
for, and a `snapshot` loop costs far more than one wait.

`attention` means the worker is asking for something — a permission prompt, a
question. Read the screen and decide. An outward action it proposes (a push,
a public comment, closing an issue) waits for the user's answer, not yours.

## Long runs and the wait cap

A wait is bounded: `timeout_ms` defaults to 30 000 and is capped at 300 000
(five minutes), on `wait_for_status` and `prompt_in_session` alike. A timeout
is not an error: the call returns `{ status, timed_out: true }` with the
session's current status. For a task that runs longer, wait again — reading
the screen between two waits shows how far it got. A session that exits
settles any wait with `exited`.

## Prompting another Claude needs an opt-in

`prompt_in_session` refuses a **Claude** target unless nesting is allowed,
either for every call:

```json
{ "mcp": { "allow_claude_nesting": true } }
```

or for one call, with `allow_claude_nesting: true` among its arguments. A
shell target is never gated. The separate `run_in_session` +
`wait_for_status` calls are not gated either: the opt-in guards the composed
call only.

## Text already in another tab's input box

Before typing into a Claude session you did not just open, read its screen.
Text already sitting in its input box is usually a **suggestion** Claude Code
offers, drawn there for the user to accept or ignore — and in plain text it
looks exactly like something the user typed. It may also be the user's own
unfinished draft. Either way, it is not yours to send: never submit it
without asking the user.

## Leave the screenshot for last

`read_terminal` answers what a session printed. `screenshot` returns the whole
window as a PNG and weighs hundreds of times a `snapshot`: take one only for a
question text cannot answer — a colour, a glyph, a layout — and at the end of
the run rather than inside a loop.

## TermHerd's own interface

`press_keys` and `run_action` drive TermHerd, not a terminal. An open prompt
takes their keys, and `escape` leaves any prompt. `enter` on the quit prompt
quits TermHerd, and with it the bridge the driving session is talking through.
See [Driving the keyboard](./keyboard.md).
