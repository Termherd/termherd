// TermHerd — Issue #23 working note (sidebar launch affordances).
// Palette borrowed from docs/notes/issue-29-os-notifications.typ for consistency.

#let bg     = rgb("#111318")
#let panel  = rgb("#181b22")
#let fg     = rgb("#d0d0d0")
#let dim    = rgb("#8a93a3")
#let accent = rgb("#5ec8a0") // terminal green
#let cyan   = rgb("#56b6c2")
#let amber  = rgb("#e5c07b")
#let red    = rgb("#e06c75")
#let mono   = "JetBrainsMono NF"

#set document(title: "TermHerd — #23 Sidebar launch", author: "working note")
#set page(width: 21cm, height: auto, margin: (x: 1.8cm, y: 1.6cm), fill: bg)
#set text(font: "Inter", size: 10.5pt, fill: fg)
#show raw: set text(font: mono, size: 0.9em)
#set par(leading: 0.6em, justify: true)
#show heading: set text(fill: accent)
#show heading.where(level: 1): set text(size: 16pt)
#show heading.where(level: 2): set text(size: 11.5pt, fill: cyan)

#let kicker(s) = text(font: mono, size: 8pt, fill: accent, tracking: 2pt, upper(s))

#kicker("issue #23 · enhancement · P1 · feat/repo-launch-buttons · RESHAPE")
= Sidebar launch affordances: a missing capability, not a missing button

#text(fill: dim)[
  One-page intent note. Read before touching the sidebar. The headline: the
  prior root-cause analysis on the issue was *wrong*, and the correction
  changes what we build.
]

== 1 · The root cause we first wrote down was false
The issue (and an earlier comment) framed #23 as a *discoverability gap*: "the
machinery exists — clicking a repo name already spawns a bare `claude` in the
cwd — it's just invisible." That premise does not survive reading the adapter.

`crates/pty/src/lib.rs:346–390` shows every `Effect::Spawn` launches the user's
*login shell* in the project dir. The word `claude` is typed into that shell in
*exactly one* case: when `SpawnSpec.resume == Some(id)`, the adapter writes
`claude --resume <id>`. With `resume == None`, nothing is typed — you get a
*bare shell*.

So the gestures today are:

#text(font: mono, size: 9pt)[
  repo name  → resume:None  → plain shell, NO claude \
  session    → resume:Some  → shell + `claude --resume <id>` \
  ▾ arrow    → collapse
]

There is *no fresh-Claude launch anywhere in the app.* #23's stated goal —
"open a fresh Claude session" — is a genuinely *missing third mode*, not an
unsurfaced existing one.

== 2 · The reshape (verdict: reshape, worth it)
Clicking the repo *name* to launch is also a poor affordance — a tree header
should fold. So we split the one overloaded gesture into three explicit ones:

#text(font: mono, size: 9pt)[
  repo name   → toggle collapse          (no launch) \
  button \$   → plain shell in cwd        (today's name action, relocated) \
  button 🤖   → fresh `claude` in cwd     (THE NEW MODE) \
  session     → resume that claude        (unchanged)
]

Icons chosen deliberately: `\$` for the shell (a `>` prompt would collide with
Claude's own invite prompt); a bot for Claude (unambiguous).

== 2b · A boundary we deliberately did *not* cross
"Open a plain shell" and "open Claude" are now two product actions. We are
*not* turning TermHerd into a general terminal launcher beyond this — the shell
button is the existing behaviour made explicit, not a new product thesis. The
question "are system / UI / Claude-interaction concerns cleanly separated as
domains?" is real but out of #23's scope — parked as a forward architectural
note, not actioned here.

== 3 · The one bit of tech scope
`SpawnSpec.resume: Option<String>` has two states; we need three. Promote it to
a launch kind so the core says *what* to launch, the adapter says *how*:

#text(font: mono, size: 9pt)[
  enum Launch { Shell, Claude { resume: Option<String> } } \
  \
  Shell                       → button \$        → adapter types nothing \
  Claude { resume: None }     → button 🤖 (new)  → adapter types `claude` \
  Claude { resume: Some(id) } → session click     → adapter types `claude --resume id`
]

Plus a tidy: `project_label` (name-a-repo-from-its-path) moves
`crates/app/src/shell/view.rs` → `crates/core`. Conceded on DRY/placement, not
"single use" — it already drives *two* call sites (sidebar label at view.rs:154,
launched-tab title at shell.rs:463), which is the project's own "hoist into core
once" trigger.

== 4 · Test surface (drive these first, red)
- `core` — the `Launch` variant `apply()` emits per gesture: Shell vs
  Claude{None} vs Claude{Some}. Non-regression: the fresh-Claude button is
  *always* `Claude{resume:None}`; repeated launch of one repo opens *distinct*
  tabs (core never dedupes).
- `core::…::project_label` (moved) — unit (nested, trailing `/`, Windows `\`,
  collapsed-worktree, bare name, `/` & `""` fallbacks) + property-based
  (last-joined-segment, trailing-separator invariance, separator-free result).
- adapter — `Claude{None}` types `claude\r` (no `--resume`); `Shell` types
  nothing. Test data from real `~/.claude/projects` paths.

== 5 · Cognitive-debt ledger (Phase 6)
- *Override (the big one).* The human refuted the AI's "every spawn is claude"
  claim against the adapter source — fresh spawn is a *bare shell*; fresh-Claude
  was missing. This reframed the issue from discoverability to missing-capability
  and corrected a wrong root-cause comment. The strongest retained-judgment
  signal of the session.
- *Override.* Reshaped the issue: repo-name → collapse, and *two* explicit
  buttons (`\$` shell, 🤖 Claude) instead of one ＋. Diverged from the prior
  plan, and from the AI's first reading.
- *Concession.* Conceded the `project_label` → `core` move on DRY/placement once
  shown two real consumers — overriding their own "keep it in view" instinct.
- *Predictions converged.* Distinguished tab titles + dedup-free launch + OSC
  retitle-for-free were all predicted and confirmed.
- *Probe (Pause B, planted error).* The AI claimed `launch_command` lives in
  `core`; it lives in the `pty` adapter (correctly — typed-string is the *how*).
  Not caught in the human's defense. The one boundary-reasoning miss in an
  otherwise high-engagement session.
- *Forward thread (not actioned).* The human flagged that system / UI / Claude-
  interaction concerns aren't cleanly separated as domains — worth a future
  challenge, out of #23's scope.
