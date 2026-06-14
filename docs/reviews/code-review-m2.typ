// TermHerd — M0→M2 code review deck
// Palette is TermHerd's own terminal default: bg #111318, fg #d0d0d0.
// CeTZ is used for three explanation diagrams (cached offline).

#import "@preview/cetz:0.4.2"

#let bg      = rgb("#111318")
#let panel   = rgb("#181b22")
#let fg      = rgb("#d0d0d0")
#let dim     = rgb("#8a93a3")
#let accent  = rgb("#5ec8a0") // terminal green
#let cyan    = rgb("#56b6c2")
#let amber   = rgb("#e5c07b")
#let red     = rgb("#e06c75")
#let mono    = "JetBrainsMono NF"

#set document(title: "TermHerd — M0→M2 Code Review", author: "Code review")
#set page(
  width: 25.4cm, height: 14.29cm, // 16:9
  margin: (x: 1.6cm, y: 1.3cm),
  fill: bg,
)
#set text(font: "Inter", size: 15pt, fill: fg)
#show raw: set text(font: mono, size: 0.92em)
#set par(leading: 0.62em)

// ── helpers ─────────────────────────────────────────────────────────────
#let kicker(s) = text(font: mono, size: 9pt, fill: accent, tracking: 2pt, upper(s))

#let chip(label, col) = box(
  fill: col.lighten(0%).transparentize(80%),
  stroke: 0.6pt + col,
  inset: (x: 6pt, y: 2pt), radius: 3pt,
)[#text(font: mono, size: 8.5pt, fill: col, weight: "bold")[#label]]

// Standard content slide with a running header.
#let slide(title: none, kick: none, body) = {
  set page(footer: context [
    #set text(font: mono, size: 7.5pt, fill: dim)
    #grid(columns: (1fr, auto),
      [termherd · m0→m2 review],
      [#counter(page).display()],
    )
  ])
  if title != none [
    #if kick != none { kicker(kick); v(-4pt) }
    #text(size: 22pt, fill: fg, weight: "bold")[#title]
    #v(-2pt)
    #line(length: 100%, stroke: 0.8pt + accent.transparentize(40%))
    #v(2pt)
  ]
  body
}

// A finding card.
#let finding(n, total, sev, sevcol, title, file, problem, scenario, fix) = {
  set page(footer: context [
    #set text(font: mono, size: 7.5pt, fill: dim)
    #grid(columns: (1fr, auto), [termherd · m0→m2 review], [#counter(page).display()])
  ])
  kicker("finding " + str(n) + " / " + str(total)); v(-4pt)
  text(size: 19pt, fill: fg, weight: "bold")[#title]
  v(-3pt)
  line(length: 100%, stroke: 0.8pt + sevcol.transparentize(40%))
  v(3pt)
  grid(columns: (auto, 1fr), gutter: 8pt,
    chip(sev, sevcol),
    align(right + horizon)[#text(font: mono, size: 9pt, fill: cyan)[#file]],
  )
  v(5pt)
  block(fill: panel, radius: 5pt, inset: 9pt, width: 100%)[
    #set text(size: 12.5pt)
    #text(fill: amber, weight: "bold")[Problem.] #problem
  ]
  v(6pt)
  grid(columns: (1fr, 1fr), gutter: 10pt, align: top + left,
    block(fill: panel, radius: 5pt, inset: 9pt, width: 100%)[
      #text(fill: red, weight: "bold", size: 10.5pt)[⚠ FAILURE SCENARIO] \
      #v(1pt) #set text(size: 11.5pt); #scenario
    ],
    block(fill: accent.transparentize(88%), radius: 5pt, inset: 9pt, width: 100%,
      stroke: 0.6pt + accent.transparentize(40%))[
      #text(fill: accent, weight: "bold", size: 10.5pt)[✓ FIX] \
      #v(1pt) #set text(size: 11.5pt); #fix
    ],
  )
}

// A diagram slide: title + one-line caption + a centered CeTZ canvas.
#let dia(kick, title, caption, canvas) = slide(kick: kick, title: title)[
  #text(size: 13pt, fill: dim)[#caption]
  #v(10pt)
  #align(center, canvas)
]

// ════════════════════════════════════════════════════════════════════════
// TITLE
// ════════════════════════════════════════════════════════════════════════
#page(footer: none)[
  #v(1fr)
  #kicker("code review · medium effort")
  #v(6pt)
  #text(size: 40pt, weight: "bold", fill: fg)[TermHerd]
  #v(-10pt)
  #text(size: 20pt, fill: dim)[M0 → M2 build-out — what changed & what to fix]
  #v(14pt)
  #grid(columns: (auto, auto, auto, auto), gutter: 8pt,
    chip("19 commits", cyan),
    chip("~9,000 LOC", cyan),
    chip("8 findings", amber),
    chip("0 crashers", accent),
  )
  #v(1fr)
  #text(font: mono, size: 9pt, fill: dim)[range 0fa45ff..HEAD · pty → gui hot path · 2026-06-13]
]

// ════════════════════════════════════════════════════════════════════════
// WHAT CHANGED
// ════════════════════════════════════════════════════════════════════════
#slide(kick: "context", title: "What landed since 0fa45ff")[
  #set text(size: 14pt)
  Three milestones took the repo from an empty scaffold to a working embedded
  terminal — built on the hexagonal rule: #text(font: mono, size: 11pt, fill: cyan)[adapters → core ← claude].

  #v(6pt)
  #grid(columns: (1fr, 1fr, 1fr), gutter: 10pt, align: top + left,
    block(fill: panel, radius: 5pt, inset: 9pt, width: 100%)[
      #chip("M0", dim) #h(4pt) *Foundations* \ #v(2pt)
      #set text(size: 11pt, fill: dim)
      Workspace skeleton, CI gates, iced window shell, cargo-dist release
      pipeline (mac/linux/win).
    ],
    block(fill: panel, radius: 5pt, inset: 9pt, width: 100%)[
      #chip("M1", cyan) #h(4pt) *Browser + search* \ #v(2pt)
      #set text(size: 11pt, fill: dim)
      `scan` adapter, pure `core::browser` grouping, debounced fs-watch (FR2),
      in-memory search (FR3).
    ],
    block(fill: panel, radius: 5pt, inset: 9pt, width: 100%)[
      #chip("M2", accent) #h(4pt) *Embedded terminal* \ #v(2pt)
      #set text(size: 11pt, fill: dim)
      `pty` actor-per-session, colour grid, raw keys, resize, OSC badges (FR8),
      scrollback + select (FR4).
    ],
  )
  #v(7pt)
  #block(fill: amber.transparentize(90%), radius: 5pt, inset: 9pt, width: 100%,
    stroke: 0.6pt + amber.transparentize(55%))[
    #set text(size: 12.5pt)
    #text(fill: amber, weight: "bold")[The pure `claude` codec] underpins it all:
    `path` · `derive` · `digest` · `osc` · `jsonl` — no I/O, fully unit + property tested.
  ]
]

// ════════════════════════════════════════════════════════════════════════
// THE VERDICT / MAP
// ════════════════════════════════════════════════════════════════════════
#slide(kick: "verdict", title: "Eight findings across two passes")[
  Pass 1 (rows 1–6) covered `0fa45ff..` — no crashers, the
  #text(fill: amber, weight: "bold")[PTY → GUI hot path]. Pass 2 (7–8) covers
  `ce13614`, the #text(fill: amber, weight: "bold")[Attention status].

  #v(4pt)
  #set text(size: 11.5pt)
  #table(
    columns: (auto, 1fr, auto, auto),
    stroke: none,
    fill: (_, row) => if row == 0 { accent.transparentize(85%) }
                       else if calc.odd(row) { panel } else { none },
    inset: (x: 8pt, y: 4pt),
    align: (center, left, left, center),
    text(font: mono, size: 9pt, fill: accent)[\#],
    text(font: mono, size: 9pt, fill: accent)[FINDING],
    text(font: mono, size: 9pt, fill: accent)[AREA],
    text(font: mono, size: 9pt, fill: accent)[SEV],

    [1], [Full grid snapshot per byte-chunk → unbounded channel], [pty], chip("HIGH", red),
    [2], [Per-cell `String` + `Advanced` shaping every frame], [gui], chip("MED", amber),
    [3], [`spawn()` swallows poisoned lock, returns `Ok`], [pty], chip("MED", amber),
    [4], [Hardcoded palette constant drifts from `pty`], [gui], chip("LOW", cyan),
    [5], [Blocking `scan()` inside an async task], [gui], chip("LOW", cyan),
    [6], [Focus + selection logic stranded in the shell], [altitude], chip("NOTE", dim),
    [7], [`Attention` is sticky with no non-`Busy` escape (payload discarded)], [pty], chip("MED?", amber),
    [8], [Status state machine split across `pty` + `core`], [altitude], chip("NOTE", dim),
  )
  #v(3pt)
  #text(size: 10pt, fill: dim)[Dropped on verify: empty-line selection panic (grid is
  padded to `cols`); `u64` SessionId overflow (~1.8 × 10¹⁹ launches).]
]

// ════════════════════════════════════════════════════════════════════════
// FINDINGS
// ════════════════════════════════════════════════════════════════════════
#finding(1, 8, "HIGH · memory + cpu", red,
  "Snapshot-per-chunk over an unbounded channel",
  "crates/pty/src/lib.rs:525  ·  main.rs:57",
  [`spawn_term` clones the *entire* visible grid (`snapshot(&term)`) and sends a
   `PtyEvent::Output` after #text(weight: "bold")[every] `TermCmd::Bytes` chunk — across an
   #text(font: mono)[unbounded] channel.],
  [`cat large_file` or a streaming `claude` reply delivers hundreds of small
   chunks/sec. If the GUI lags (resize, shader compile) the queue grows without
   bound → memory blowup. Even keeping up, every snapshot but the last per frame
   is thrown away — O(rows·cols) wasted per chunk.],
  [Coalesce: emit at most one snapshot per frame (timer / dirty flag). Switch to
   a #text(font: mono)[bounded] channel that drops stale frames under backpressure.
   #text(fill: accent, weight: "bold")[Highest leverage — fixes both axes at the root.]],
)

#dia("finding 1 · diagram", "Why the queue grows without bound",
  [Every read chunk clones the whole grid into an *unbounded* channel. The fix:
   one snapshot per frame, into a bounded channel that drops stale frames.],
  cetz.canvas(length: 0.82cm, {
    import cetz.draw: *
    let box(x, y, w, body, sc, tc) = {
      rect((x, y), (x + w, y + 1.2), fill: panel, stroke: sc + 0.9pt, radius: 0.12)
      content((x + w/2, y + 0.6), text(font: mono, size: 8.5pt, fill: tc)[#body])
    }
    let arr(x1, y, x2, c) = line((x1, y), (x2, y), mark: (end: ">", fill: c), stroke: c + 1pt)

    content((-0.2, 5.7), anchor: "west", text(font: mono, size: 9pt, fill: red, weight: "bold")[BEFORE])
    box(0, 4.2, 2.6, [PTY child], dim, fg)
    arr(2.6, 4.8, 3.5, dim)
    box(3.5, 4.2, 3.0, [reader thd], dim, fg)
    arr(6.5, 4.8, 7.4, dim)
    box(7.4, 4.2, 3.8, [term \ snapshot()], amber, amber)
    arr(11.2, 4.8, 12.1, red)
    for i in range(6) {
      rect((12.2 + i*0.3, 4.0 + i*0.16), (13.9 + i*0.3, 5.4 + i*0.16),
        fill: red.transparentize(72%), stroke: red + 0.6pt, radius: 0.05)
    }
    content((13.9, 6.5), text(font: mono, size: 7.5pt, fill: red)[unbounded ↑])
    arr(15.7, 4.8, 16.6, dim)
    box(16.6, 4.2, 3.0, [GUI (lags)], dim, fg)
    content((9.3, 3.6), text(size: 8pt, fill: amber)[full grid clone / chunk])

    content((-0.2, 1.6), anchor: "west", text(font: mono, size: 9pt, fill: accent, weight: "bold")[AFTER])
    box(7.4, 0.2, 3.8, [term \ coalesce 1/frame], accent, accent)
    arr(11.2, 0.8, 12.1, accent)
    for i in range(3) {
      rect((12.2 + i*0.55, 0.3), (12.7 + i*0.55, 1.3),
        fill: accent.transparentize(72%), stroke: accent + 0.7pt, radius: 0.05)
    }
    content((13.0, 2.0), text(font: mono, size: 7.5pt, fill: accent)[bounded(N), drop stale])
    arr(14.1, 0.8, 16.6, accent)
    box(16.6, 0.2, 3.0, [GUI], accent, accent)
  }))

#finding(2, 8, "MED · cpu + alloc", amber,
  "Per-cell allocation in the canvas draw loop",
  "crates/app/src/shell.rs:589",
  [Each redraw calls `cell.c.to_string()` per non-blank cell and renders with
   `Shaping::Advanced` — for single monospace glyphs.],
  [An 80×24 grid is ~1,900 heap allocations *plus* advanced text-shaping per
   frame, multiplied by frame rate during active output. Pure overhead in the
   hottest rendering path; compounds finding 1.],
  [Use `Shaping::Basic` for monospace single-char cells and avoid the per-cell
   `String`. Cheaper alloc + cheaper shaping, identical output.],
)

#finding(3, 8, "MED · correctness", amber,
  "spawn() swallows a poisoned lock and returns Ok",
  "crates/pty/src/lib.rs:354",
  [`if let Ok(mut map) = self.sessions.lock()` skips the `insert` on a poisoned
   mutex but the function still returns `Ok(())`.],
  [After a panic poisons the lock, the reader/term threads run #text(style: "italic")[detached]
   while the session is never registered. Every later `write` / `resize` / `kill`
   returns `NoSuchSession` — a live-looking but dead tab.],
  [Surface the poison as `PtyError::Io` like `write()`/`resize()` already do
   (lib.rs:364/381) — or tear the orphaned threads down. Make the three lock
   sites consistent.],
)

#dia("finding 3 · diagram", "Same poison, two different answers",
  [On a poisoned lock, `write()`/`resize()` return an error — but `spawn()`
   swallows it and returns `Ok`, leaving threads running but unreachable.],
  cetz.canvas(length: 0.82cm, {
    import cetz.draw: *
    let box(x, y, w, h, body, fc, sc, tc) = {
      rect((x, y), (x + w, y + h), fill: fc, stroke: sc + 0.9pt, radius: 0.12)
      content((x + w/2, y + h/2), text(font: mono, size: 8pt, fill: tc)[#body])
    }
    let down(x, y1, y2, c) = line((x, y1), (x, y2), mark: (end: ">", fill: c), stroke: c + 1pt)
    let branch(x1, y1, x2, y2, c) = line((x1, y1), (x2, y2), mark: (end: ">", fill: c), stroke: c + 1pt)

    box(6.1, 7.2, 4.0, 1.1, [sessions.lock()], panel, dim, fg)
    content((10.4, 7.75), anchor: "west", text(size: 8pt, fill: dim)[returns Err — mutex poisoned])
    down(8.1, 7.2, 6.7, dim)
    content((8.1, 6.35), text(font: mono, size: 8.5pt, fill: amber)[poisoned?])
    branch(7.2, 6.05, 4.0, 4.65, red)
    branch(9.0, 6.05, 12.4, 4.65, accent)

    content((2.4, 5.0), text(font: mono, size: 9pt, fill: red, weight: "bold")[spawn()])
    box(1.6, 3.4, 4.8, 1.2, [skip insert \ return Ok(())], red.transparentize(82%), red, red)
    down(4.0, 3.4, 2.8, red)
    box(0.8, 1.4, 6.4, 1.3, [threads run detached; \ later write/resize/kill \ → NoSuchSession], panel, red, fg)

    line((8.4, 1.2), (8.4, 5.8), stroke: (paint: dim, dash: "dashed", thickness: 0.6pt))

    content((12.4, 5.0), text(font: mono, size: 9pt, fill: accent, weight: "bold")[write() · resize()])
    box(10.0, 3.4, 4.8, 1.2, [return \ PtyError::Io], accent.transparentize(82%), accent, accent)
    content((12.4, 2.7), text(size: 8pt, fill: accent)[✓ caller learns it failed])
  }))

#finding(4, 8, "LOW · cleanup", cyan,
  "Hardcoded palette constant drifts from pty",
  "crates/app/src/shell.rs:584",
  [Render skips the default background via `cell.bg != [0x11,0x13,0x18]` —
   a magic literal duplicating private `pty::DEFAULT_BG` (and cursor `DEFAULT_FG`).],
  [Change pty's default palette and the shell keeps comparing the old value: every
   cell paints its background (or the wrong cells skip) with #text(style: "italic")[no compile
   error]. Silent visual drift.],
  [Make `DEFAULT_BG` / `DEFAULT_FG` `pub` in the `pty` crate and reference them
   from the shell. One source of truth.],
)

#finding(5, 8, "LOW · responsiveness", cyan,
  "Blocking scan() inside an async task",
  "crates/app/src/shell.rs:184",
  [`Task::perform(async move { scanner.scan() … })` runs a synchronous fs walk +
   JSONL parse directly inside an `async` block.],
  [On a large `~/.claude/projects` tree the blocking walk ties up an iced executor
   worker for its whole duration on every debounced rescan (FR2), delaying the
   `ScanCompleted` result.],
  [Wrap the blocking work in `spawn_blocking` (or a dedicated thread) so the
   executor stays free. Idiomatic async hygiene.],
)

#finding(6, 8, "NOTE · altitude", dim,
  "Domain logic stranded in the GUI shell",
  "crates/app/src/shell.rs:99 (Focus, selection_text)",
  [The `Focus` enum (input routing) and `selection_text` / `selection_span`
   (FR4) live in `shell.rs`, not in pure `core`.],
  [They can't be unit-tested headlessly, contradicting AGENTS.md ("domain logic
   lives behind `apply`"). When tabs/splits land (FR5+), per-pane focus and
   selection must be duplicated or retrofitted into `core`.],
  [Move focus routing into `core::Workspace` and selection into `core` (or a
   tested helper). Pay the altitude cost now, before splits multiply it.],
)

#dia("finding 6 · diagram", "Logic living in the wrong layer",
  [Input routing and selection are domain decisions sitting in the GUI shell.
   The hexagonal rule wants them in pure, testable `core`.],
  cetz.canvas(length: 0.66cm, {
    import cetz.draw: *
    let band(y, h, fc, sc, label, sub) = {
      rect((0, y), (18, y + h), fill: fc, stroke: sc + 1pt, radius: 0.15)
      content((0.4, y + h - 0.45), anchor: "west", text(font: mono, size: 9pt, fill: sc, weight: "bold")[#label])
      content((0.4, y + 0.4), anchor: "west", text(size: 7.5pt, fill: dim)[#sub])
    }
    let chip(x, y, w, body, c) = {
      rect((x, y), (x + w, y + 0.95), fill: c.transparentize(78%), stroke: c + 0.8pt, radius: 0.1)
      content((x + w/2, y + 0.48), text(font: mono, size: 8pt, fill: c)[#body])
    }

    band(7.0, 2.6, panel, cyan, "app / shell  (iced GUI)", "translates events, performs effects")
    chip(9.5, 7.55, 3.6, [Focus enum], red)
    chip(13.4, 7.55, 4.2, [selection_text()], red)

    band(3.6, 2.6, accent.transparentize(90%), accent, "core  (pure state machine)", "App::apply — headless, unit-testable")
    content((13.0, 4.9), text(size: 8pt, fill: accent)[← input routing + selection belong here])

    band(0.6, 2.0, panel, dim, "claude  (pure codec)", "path · derive · digest · osc · jsonl")

    line((11.3, 7.5), (11.3, 6.2), mark: (end: ">", fill: amber), stroke: (paint: amber, thickness: 1.2pt))
    content((11.3, 6.85), anchor: "west", text(font: mono, size: 8pt, fill: amber)[ move])
  }))

// ════════════════════════════════════════════════════════════════════════
// PASS 2 — new code (ce13614: Attention status + sidebar badges)
// ════════════════════════════════════════════════════════════════════════
#finding(7, 8, "MED? · ux — contingent", amber,
  "Attention is sticky with no non-Busy escape",
  "crates/pty/src/lib.rs · fold_status",
  [*From the code:* any OSC 9 `Notification` → `Attention`; only `Busy` clears
   it (not `Idle`); and `fold_status` discards the payload, so notification
   types are indistinguishable.],
  [*Contingent:* a "done"-type OSC 9 (if Claude sends one) leaves the row red
   until the next `Busy` — maybe never, with no escape path. For a session
   truly awaiting input, sticky is correct. So: a bug only given Claude's real
   OSC 9 usage — which I did *not* verify.],
  [First capture the OSC 9 payloads Claude actually emits. If non-attention
   pings exist: branch on `Notification(payload)`, or let a focus/click
   acknowledge `Attention` → `Idle`.],
)

#dia("finding 7 · diagram", "Where Attention gets stuck",
  [`Busy`⇄`Idle` track work. An OSC 9 ping jumps to `Attention`, which `Idle`
   can't leave — only `Busy` clears it. The risk lands *only if* non-permission
   pings exist.],
  cetz.canvas(length: 0.82cm, {
    import cetz.draw: *
    let node(x, y, label, c) = {
      circle((x, y), radius: 1.05, fill: c.transparentize(82%), stroke: c + 1.1pt)
      content((x, y), text(font: mono, size: 8.5pt, fill: c, weight: "bold")[#label])
    }
    let arr(a, b, c) = line(a, b, mark: (end: ">", fill: c), stroke: c + 1pt)

    node(4.5, 4.6, [Busy], amber)
    node(11.5, 4.6, [Idle], accent)
    node(4.5, 1.3, [Atten-\ntion], red)

    arr((5.55, 4.9), (10.45, 4.9), dim)
    content((8.0, 5.25), text(font: mono, size: 7.5pt, fill: dim)[idle title])
    arr((10.45, 4.3), (5.55, 4.3), dim)
    content((8.0, 3.95), text(font: mono, size: 7.5pt, fill: dim)[busy title])

    arr((4.05, 3.55), (4.05, 2.35), red)
    content((3.1, 3.0), text(font: mono, size: 7.5pt, fill: red)[OSC 9])
    arr((4.95, 2.35), (4.95, 3.55), accent)
    content((6.0, 3.0), text(font: mono, size: 7.5pt, fill: accent)[Busy ✓])

    arr((10.6, 3.7), (5.5, 1.75), red)
    content((8.9, 2.95), text(font: mono, size: 7.5pt, fill: red)[OSC 9])

    content((4.5, -0.15), text(font: mono, size: 7.5pt, fill: red)[↺ Idle keeps it here])

    rect((8.4, 0.1), (17.6, 2.5), fill: red.transparentize(86%), stroke: red + 0.7pt, radius: 0.12)
    content((13.0, 1.3), text(size: 8.5pt, fill: fg)[#box(width: 8.4cm)[
      *Risk (if "done" pings exist):* such a ping with no later `Busy` stays red
      — and the payload that could say "done" vs "permission" is thrown away.
    ]])
  }))

#finding(8, 8, "NOTE · altitude", dim,
  "The status state machine is split across two crates",
  "pty · fold_status  +  core · StatusChanged",
  [The rule "`Attention` is sticky and outranks `Idle`" lives in `pty`'s
   `fold_status`; the rule "`Exited` is terminal" lives in `core`'s
   `StatusChanged`. No single place holds the whole machine.],
  [`core::apply` blindly accepts whatever status `pty` computed (except after
   `Exited`), so the stickiness invariant can't be unit-tested in `core` and
   can't be enforced if a second status producer ever appears. Reinforces
   finding 6: domain rules leaking out of `core`. (Safe today — `pty` is the
   sole producer and mirrors `core`.)],
  [Move the transition rules (rank + stickiness) into a pure `core` helper that
   `StatusChanged` calls; leave `pty` to emit raw signals. The whole machine
   becomes one tested unit.],
)

// ════════════════════════════════════════════════════════════════════════
// PLAN
// ════════════════════════════════════════════════════════════════════════
#slide(kick: "changes to make", title: "Recommended order")[
  #v(2pt)
  #grid(columns: (auto, 1fr), row-gutter: 9pt, column-gutter: 12pt,
    chip("1 · DO FIRST", red),
    [*Coalesce snapshots + bounded channel* (#1). Then *cheap canvas cells* (#2).
     Together these fix the hot path — the one change with real user impact.],

    chip("2 · QUICK WINS", amber),
    [*Consistent lock handling in `spawn()`* (#3) and *exported palette
     constants* (#4) — small, low-risk diffs. Plus *confirm Claude's OSC 9
     usage* (#7) before deciding whether Attention needs an escape path.],

    chip("3 · HYGIENE", cyan),
    [*`spawn_blocking` for `scan()`* (#5). One-line idiomatic fix.],

    chip("4 · BEFORE M3", dim),
    [*Lift focus + selection into `core`* (#6) and *fold the status machine into
     `core`* (#8). Do it before tabs/splits make the duplication permanent.],
  )
  #v(10pt)
  #block(fill: accent.transparentize(88%), radius: 5pt, inset: 11pt, width: 100%,
    stroke: 0.6pt + accent.transparentize(40%))[
    #text(fill: accent, weight: "bold")[Next step.] Findings #1 + #2 are the
    highest-leverage pair. Say the word and I'll apply them to the working tree.
  ]
]

#page(footer: none)[
  #v(1fr)
  #align(center)[
    #kicker("end")
    #v(8pt)
    #text(size: 22pt, weight: "bold")[Questions / pick the fixes to apply]
    #v(6pt)
    #text(font: mono, size: 10pt, fill: dim)[termherd · m0→m2 · medium-effort review]
  ]
  #v(1fr)
]
