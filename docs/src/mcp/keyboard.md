# Driving the keyboard

Two tools reach TermHerd's **own interface** — the sidebar, the tab strip, the
overlays — rather than a terminal. Typing *into* a session stays
`run_in_session`'s job.

| Tool | Takes | Tests |
| --- | --- | --- |
| `press_keys` | chords in `settings.json` syntax — `"cmd+shift+s"`, `"ctrl+tab"`, `"escape"` | the **binding**, resolved through the *live* keymap, including your overrides |
| `run_action` | kebab-case action names — `"split-vertical"`, `"activate-tab-3"` | the **behaviour**, skipping the keymap, so it survives a rebind |

Both take a list applied in order and return one `steps` entry per item, plus
the resulting `focused_handle`.

## Why it is a real key event

A chord is dispatched as a **synthesised key event** fed to the app's key
handler — the whole routing ladder, not just a keymap lookup.

That is what makes <kbd>Escape</kbd> and <kbd>Enter</kbd> reachable at all:
they are *overlay* keys, bound to no action. Without them an agent that armed a
close confirmation would have no way to answer it, and would park the app until
a human intervened.

The two are not equally complete, and the difference matters to a caller.
**`escape` leaves every prompt** — that is what guarantees an agent can always
back out. **`enter` confirms the confirmation prompts but commits neither
rename**, because both commit through a widget callback no synthesised event
reaches ([#246](https://github.com/Termherd/termherd/issues/246)).

The corollary: **an open overlay consumes an MCP press exactly as it consumes a
keypress.** The step reports which prompt ate it, so a caller learns why its
chord did nothing. `run_action` is gated on the same ladder on purpose —
neither tool may reach a state the keyboard cannot.

## Reading a step

| Outcome | Means | Extra field |
| --- | --- | --- |
| `ran` | the ladder applied it | `action` — the name that ran |
| `inert` | nothing happened | `reason` — see below |
| `overlay` | an open prompt consumed it | which prompt |
| `typed` | bound to nothing, so it reached the focused terminal | |
| `unbound` | nothing claimed it | |

`ran` means **the shell applied the event**, not that the effect was
interesting: `activate-tab-9` on a single-tab workspace reports `ran`, because
the event was applied and absorbed. Collapsing that into `inert` would make the
distinction useless.

### The two kinds of nothing

`inert` carries a `reason` because they call for opposite responses:

| Reason | Means | Do |
| --- | --- | --- |
| `no-surface` | the action is wired to nothing yet (`open-new-session` is the one) | **stop** — retrying is pointless |
| `no-context` | a precondition was absent — nothing focused to derive a repo from, no closed tab to reopen, nothing to scroll, nothing selected to copy | **create it**, then retry |

Seven handlers can refuse this way, and each says so at its own refusal site.

## Answering an overlay

`escape` usually cancels; `enter` usually confirms. Three cautions:

- On **`quit-confirm`**, `enter` quits the app — killing every session and the
  connection you are speaking over.
- **`session-rename`** (the sidebar's inline ✎ field) answers to neither
  `enter` nor the rename in the doc pane: both commit through the widget's own
  submit, which a synthesised key event never reaches. `escape` abandons it —
  so you can always back out and start over — but committing a rename over MCP
  is a missing capability, tracked as
  [#246](https://github.com/Termherd/termherd/issues/246).
- Every other overlay is exitable from the keyboard, and a test sweep derived
  from the overlay enumeration — not a hand-written list — is what keeps it
  that way. A new overlay added without an exit fails there.

## Rejection is all-or-nothing

A malformed chord or an unknown action name rejects the **whole call** before
anything applies. The error names the offender and the syntax. This is
deliberate: a half-applied sequence is worse than none, because the caller
cannot tell how far it got.

The action catalogue is published by the [stdio server](./stdio.md) at
`termherd://keys/schema`; the live bridge serves tools only, so its `run_action`
error message carries the syntax instead.

## Example: verify a gesture end to end

```text
run_action(["split-vertical"])        → { steps: [{ outcome: "ran",
                                                    action: "split-vertical" }],
                                          focused_handle: "4" }
screenshot({ max_width: 900 })        → the pixels, to check the divider
press_keys(["cmd+w"])                 → { steps: [{ outcome: "overlay",
                                                    overlay: "close-confirm" }] }
press_keys(["escape"])                → cancelled
```
