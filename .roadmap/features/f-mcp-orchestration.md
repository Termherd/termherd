+++
id = "F-mcp-orchestration"
type = "feature"
area = ["mcp", "workspace"]
status = "done"
target = ["Could"]
+++

The action rung: six mutating tools, each over an existing `core::App` event.

**The action rung.** Six mutating tools — `open_session`, `split_pane`,
`focus_pane`, `rename_tab`, `close_pane`, `run_in_session` — each a thin
wrapper over an existing `core::App` event (never a new state path, the #90
constraint). `core` is untouched: the bridge grows an `Act(Action)` request +
`ActionOutcome` reply, and the shell adapter (which owns `core::App` **and**
the one effect executor) resolves the stable handle, applies the event(s)
through `App::apply`, performs the effects, and reports the **resulting focused
handle** — so an agent gets act→observe in one round trip. A handle no open
pane hosts / an out-of-range tab is rejected before any state is touched
(surfaced as `invalid_params`); every call stays `tokio::timeout`-bounded and
apply-and-read (Q7). Handles are strings, matching `list_sessions`/`snapshot`,
and address a pane **in any tab** — `Event::RevealPane` activates the owning
tab first, since click-to-focus (`FocusPane`) only reaches the active one and a
silent no-op there would let a close destroy the wrong terminal. Depends on #193;
with #212 (perception) this closes the act→observe loop
