+++
id = "F-mcp-keys"
type = "feature"
area = ["mcp", "keymap"]
status = "done"
target = ["Could"]
+++

The keyboard rung: drive the app by key chords through the real keymap.

**The keyboard rung.** Drive the *app* by key chords, not a terminal, so
capture/record, fold, tab cycling and any future binding become reachable
without a tool per gesture. Two tools over one dispatch, because they answer
different questions: `press_keys` takes chords in `settings.json` syntax and
resolves them through the **live** keymap (testing the *binding*, the user's
overrides included), `run_action` takes the kebab-case names the stdio server
already publishes at `termherd://keys/schema` and skips the keymap (testing the
*behaviour*, surviving a rebind). The design decision that earned its keep: a
chord goes in as a **synthesised key event** through the real `on_key` ladder
rather than resolved straight to its `Action` — `escape` and `enter` are
*overlay* keys bound to no action, so the cheaper path would have let an agent
arm a close-confirmation it could never answer, parking the app until a human
intervened. The corollary is faithful in both directions: an open prompt
consumes an MCP press exactly as it consumes a keypress, and the step names the
prompt so the caller learns why. `run_action` is gated on the same ladder
deliberately — neither tool may reach a state the keyboard cannot. Each press
answers `ran` / `inert` / `overlay` / `typed` / `unbound` plus the resulting
`focused_handle`; a malformed chord or unknown name rejects the whole call
before anything applies, since a half-applied sequence cannot be reasoned
about. `inert` came out of reviewing the rung against its own contract:
`open-new-session` is in the keymap vocabulary and still unwired, so reporting
`ran` would have an agent record a gesture it never made — and, verifying a
fix, read a false pass. Live testing, then an adversarial review, showed the
hole was far wider than one unwired action: **seven** handlers refuse before
acting (`new_claude_here` without a focused repo, `reopen_closed_tab` on an
empty close stack, `scroll_focused` with nothing focused, `cycle_tab` with
nothing open, `toggle_record` mid-drain, `close_focused_pane` with no tab at
all, `copy_selection` with nothing selected) and every one of them reported
`ran`. The last two are the ones a caller would be hurt by soonest: a false
`ran` on `copy` has an agent paste stale clipboard content, and `close-focused`
is the very action the overlay behaviour rests on. So `inert` carries a
`reason` — `no-surface` (wired to nothing, retrying is pointless) or
`no-context` (a precondition the caller can create) — since the two call for
opposite responses. `run_action` returns `Result<Task, Inertia>` and each
refusing handler returns `Option`, so the knowledge lives *at the refusal*
rather than in a predicate here that would duplicate the list — the same smell
the tidy-first pass had just removed. The line held deliberately:
`activate-tab-9` on one tab stays `ran`, because `core` applied the event and
absorbed it. It is whether the shell refused, not whether the effect was
interesting. Tidy-first prerequisite: the overlay ladder was stated twice — as
`overlay_key`'s precedence chain and as `accepts_terminal_input`'s conjunction
— and this rung would have been a third reading, so it became one
`keyboard_owner()` predicate, with `on_key` returning a `KeyVerdict` the real
keypress discards and the tool reports (one decision, two readers, as #216 did
for the snapshot). `core` gained `Action::name()`, total where `config_name()`
was partial, so `activate-tab-N` round-trips too. Depends on #193/#194

**#237 — the corollary the design had not actually paid.** The rung's whole
argument was that an agent must be able to answer any prompt it can arm, and
one rung of the ladder did not: an open sidebar session-rename took the
keyboard and replied to no key, so every press answered `overlay` and none
could lift it. Mouse-only, which is no one, over MCP. The fix is two lines;
what closed it for good is the test shape. Asserting per prompt would have
fixed the reported one and left the class open, so the sweep is driven off a
`KeyboardOwner::ALL` sitting against a compiler-checked `match` — and it
immediately named a second offender nobody had filed, the doc editor, closable
only by its own button. It also accumulates rather than stopping at the first
failure: a sweep that names one offender reads as *and the rest are fine*,
which is the assumption that let this one through. Tidy-first prerequisite:
`PaneRename` announced itself as `session-rename` in its own `label()`, in the
MCP reply, and in the tests — the code was the only name that disagreed. Left
open on purpose: `enter` reaches neither rename over MCP, since both commit
through the widget's `on_submit`, which no synthesised key event touches. That
is a missing capability, not a parked surface — `escape` always gets the
keyboard back — so it is a separate ticket (#246) rather than a passenger on
this fix
