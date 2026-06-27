// TermHerd — Issue #100 working note (macOS Cmd+Q bypasses the quit handler).
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

#set document(title: "TermHerd — #100 macOS Cmd+Q", author: "working note")
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

#kicker("issue #100 · bug · P1 · fix/100-cmdq-quit")
= macOS Cmd+Q bypasses the quit handler

#text(fill: dim)[
  One-page working note — the root cause, a severity correction, the chosen
  fix and *why it has no headless red test*. Read before touching the quit
  path.
]

== 1 · Root cause (proven from winit source)
winit 0.30 installs a default macOS menu bar
(`winit/src/platform_impl/macos/menu.rs`) whose *Quit* item is wired to the
selector `terminate:` with the ⌘Q key equivalent. So Cmd+Q is swallowed by
AppKit's menu and runs `[NSApp terminate:]` directly — the process dies
*before* winit's event loop or our `keyboard::listen()` subscription sees
anything. That is why the #75 handler never fires on Cmd+Q.

Two facts from the winit source pin the behaviour:
- `window_delegate.rs:147` — `windowShouldClose:` queues `CloseRequested` and
  returns `false` (the close button never closes directly; it asks us).
- `app_state.rs` implements only `applicationWillTerminate:` (a teardown
  *notification*), *never* the vetoable `applicationShouldTerminate:`. Nothing
  intercepts `terminate:`.

== 2 · Severity correction
The issue feared two consequences. Only one survives scrutiny:
- #chip("real", red) *no confirm modal, no graceful `iced::exit`* — live Claude
  sessions are hard-killed on Cmd+Q with zero warning.
- #chip("unlikely", dim) *process survives / holds the single-instance lock* —
  `terminate:` genuinely ends the process, releasing the `flock` on death. No
  code path lets it linger, release `.app` included. The #75 *survival* symptom
  is not expected to reproduce via Cmd+Q.

So #100 is a *confirmation-bypass* bug, not a lock-leak bug.

== 3 · The fix — converge, don't fork
iced 0.14 builds the winit event loop internally (`iced_winit/src/lib.rs:79`)
and exposes no hook, so winit's own `with_default_menu(false)` is unreachable
without forking iced. Every viable interception is AppKit FFI.

#chip("decision", amber) *Repoint the Quit item's action `terminate:` →
`performClose:`* (keep ⌘Q). Then Cmd+Q, the menu Quit click, and the red close
button all reach winit's `windowShouldClose:` and arrive as the *same*
`CloseRequested` event — one seam, already tested. Chosen over "strip ⌘Q +
keyboard handler" (forks a second quit path, leaves menu-click unguarded) and
over a custom objc2 target class (needs cross-loop message plumbing).

```text
 red close button ─┐
 menu-click Quit   ┤ Quit item action = performClose:   (objc2, once at startup)
 Cmd+Q             ┘        │
                            ▼  windowShouldClose:  (winit)
                    Message::Window(CloseRequested)      ◄ ONE seam
                            ▼
                    request_quit → quit_intent
                    ├─ Exit    → iced::exit()            (no live sessions)
                    └─ Confirm → arm modal               (live sessions)
```

== 4 · Where the code lives (CUPID)
- #chip("safe", accent) `shell.rs::request_quit(id)` — new named convergence
  method; the `CloseRequested` branch now delegates to it. *Predictable*: one
  place every quit funnels through. Pure, no FFI.
- #chip("unsafe", red) `crate::macos` — a `#[cfg(target_os = "macos")]` module
  with a *scoped* `#[allow(unsafe_code)]` (workspace `unsafe_code = "deny"`
  stays). One fire-once fn: `mainMenu` → find the Quit item → `setAction:` /
  `setTarget:`. Quarantined mechanism; carries no policy. objc2 / objc2-app-kit
  added macOS-target-only (already transitive via winit).
- #chip("wiring", cyan) `on_window_event` — `window::Event::Opened` arm calls
  the macOS fn once, on the main thread. *Not the boot closure*: iced builds the
  app state via `program::Instance::new` *before* `run_app`
  (`iced_winit/src/lib.rs:105`), so the boot closure runs ahead of winit's
  `applicationDidFinishLaunching` — the menu doesn't exist yet and the repoint
  would silently no-op. `Opened` arrives once the event loop is running and the
  menu is installed. No-op stub on non-macOS.

== 5 · Test surface — and an honest gap
#chip("green", accent) `cmd_q_routes_through_the_same_seam_as_the_close_button`
— pins that a live session arms the confirm modal via `request_quit`, the same
seam `CloseRequested` uses; a future change cannot split off a second,
unguarded quit path. Joins the existing #75 guards (exit-when-idle,
confirm-when-live, confirm-consumes).

#chip("gap", amber) *No red unit test for the fix itself.* The behavioural
change is entirely the AppKit menu repoint — FFI, not headless-testable, and it
routes into an *already-green* `CloseRequested` path. Forcing a red test here
would be ceremony (YAGNI). Correctness is verified by *running the app*
(`cd` to the worktree, `TMPDIR=$(mktemp -d) RUST_LOG=info cargo run -p
termherd-app` to dodge the single-instance lock of a release build):
1. launch, open ≥1 live Claude session, press *Cmd+Q* → confirm modal appears;
2. with no live session, *Cmd+Q* → clean exit (no lingering process / lock);
3. menu *TermHerd ▸ Quit* → same confirm flow as Cmd+Q;
4. *minimised-window guard* — *Cmd+M* to minimise the sole window, then *Cmd+Q*:
  Quit must still fire (not beep). This is the no-key-window case the explicit
  menu-item target fixes; with a nil target NSMenu auto-enabling would disable
  Quit here.

== 6 · State at this note
*Implemented + reviewed.* Tidy-first `request_quit` extraction, the convergence
guard, the objc2 `macos` module, its deps, and the `Opened` wiring landed; a
self-review pass then pinned the Quit item's target to the app window (the
no-key-window fix in §5 step 4), quietened a duplicate-`Opened` warning, and
logged the off-main-thread skip. All gates green: `fmt`, `clippy -D warnings`,
full workspace tests (89 app + core/claude/scan/pty), `cargo deny`,
markdownlint. Live-verified: the repoint fires at launch and targets a window;
Cmd+Q with a live session → confirm modal; with none → clean exit. Outstanding:
the *minimised-window* check (§5 step 4) — can't be driven headlessly.

== 7 · Docs consulted
`AGENTS.md` (the `unsafe_code = "deny"` invariant + the `cfg(target_os)`
precedent in `main.rs::lock_name`); issue #75 / #80 / #101 (the window-close
path already routes through the confirm flow); `ARCHITECTURE.md` §8 (effects
performed by the iced shell). winit 0.30.13 + iced_winit 0.14.0 sources read
directly for the menu / close / terminate behaviour.
