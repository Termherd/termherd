// TermHerd — Issue #29 working note (OS notification forwarding).
// Palette borrowed from docs/reviews/code-review-m2.typ for consistency.

#let bg     = rgb("#111318")
#let panel  = rgb("#181b22")
#let fg     = rgb("#d0d0d0")
#let dim    = rgb("#8a93a3")
#let accent = rgb("#5ec8a0") // terminal green
#let cyan   = rgb("#56b6c2")
#let amber  = rgb("#e5c07b")
#let red    = rgb("#e06c75")
#let mono   = "JetBrainsMono NF"

#set document(title: "TermHerd — #29 OS Notifications", author: "working note")
#set page(width: 21cm, height: auto, margin: (x: 1.8cm, y: 1.6cm), fill: bg)
#set text(font: "Inter", size: 10.5pt, fill: fg)
#show raw: set text(font: mono, size: 0.9em)
#set par(leading: 0.6em, justify: true)
#show heading: set text(fill: accent)
#show heading.where(level: 1): set text(size: 16pt)
#show heading.where(level: 2): set text(size: 11.5pt, fill: cyan)

#let kicker(s) = text(font: mono, size: 8pt, fill: accent, tracking: 2pt, upper(s))
#let chip(label, col) = box(
  fill: col.transparentize(80%), stroke: 0.6pt + col,
  inset: (x: 5pt, y: 2pt), radius: 3pt,
)[#text(font: mono, size: 8pt, fill: col, weight: "bold")[#label]]

#kicker("issue #29 · enhancement · P1 · feat/os-notifications")
= Forward terminal notifications to the OS notification centre

#text(fill: dim)[
  One-page working note — why this is a *seam* not a bug, where the logic
  lives, and the test surface. Read before touching the notify path.
]

== 1 · The shape of the work
This is an *enhancement*, not a defect — there is no root-cause to excise.
OSC 9 is already decoded into `OscSignal::Notification(body)` and folded into
the sticky in-app `Attention` status (badge + dots). The *payload text* is
dropped today in `pty::fold_status`. #29 keeps that in-app path untouched and
adds a *second, parallel channel* that carries the text out to the desktop
notification centre.

== 2 · Integration point & the dependency rule
The notification *title* — "which session wants me" — needs session/tab state
that only `core` holds. So the decision is a pure `core::App::apply` rule, not
adapter glue. This is the hexagonal rule paying off: policy is testable, the
adapter stays dumb.

```text
OSC 9 ─► OscSignal::Notification(body)              claude   unchanged
            ├─► fold_status → Attention              pty      UNCHANGED (guarded)
            └─► PtyEvent::Notification{session,body} pty      new plumbing
                  └► Event::SessionNotified ─► core::apply
                        └─► Effect::Notify{ title, body }   ◄ tested policy
                              └► notify(title, body)        app  free fn (≈ #28)
```

#chip("decision", amber) Performed by a free `notify()` fn in `app`, mirroring
#28's `Effect::OpenUrl → open_url()`. *Not* a port trait: `core` never calls
it directly, and the OS-handoff precedent already exists — YAGNI.

#chip("decision", amber) Cross-platform backend (`notify-rust`) is wired only
in the implementation step; the red tests never depend on it.

== 3 · Policy rules (the part worth testing)
- *Title* = the session's current tab title, so it tracks OSC-24 renames.
- *Blank body* → default `"Claude needs your attention"`.
- *Unknown* or *exited* session → notification dropped (nothing to return to).
- The `Attention` status fold is *orthogonal* and must not change.

== 4 · Test surface
#chip("red", red) `core` — `SessionNotified → Notify`: title/body mapping,
blank-body fallback, unknown/exited drop, title-follows-rename; + 2 property
tests (body & title preserved for any live session; unknown always dropped).

#chip("green", accent) `claude` (adjacent guard) — OSC 9 body keeps inner `;`;
empty payload is still a `Notification`.

#chip("green", accent) `pty` (adjacent guard) — `fold_status` still resolves
OSC 9 → `Attention`, body never leaks into the title.

== 5 · Docs consulted
PRD §FR8 / `F-status-notifications`; `ROADMAP.md` (line ~50, OSC 9 → distinct
`Attention`); `ARCHITECTURE.md` §5 (headless core) & §8 (effects performed by
the iced shell). The PRD's risk table names `notify` as the cross-platform
pick — read it as `notify-rust` (the desktop-notification crate; `notify` is
the fs-watch crate).

== 6 · State at this note
Types + stubbed `apply` arm + `Workspace::session_title` getter landed so the
suite *compiles*; the four positive `core` tests are *red*, adjacent guards
*green*. Next: implement the `apply` policy, the `pty` body channel, the app
wiring, and the real `notify-rust` backend.
