# Brainstorm: Project name (replace `agentmux`)

| Field | Value |
| --- | --- |
| **Date** | 2026-05-28 |
| **Duration** | ~25 min |
| **Participants** | Bastien + AI Facilitator |
| **Problem shape** | Naming (forced rename: `agentmux` taken on crates.io) |

## Context

`agentmux` is the cargo working name for the Rust replatform of an Electron
Claude Code session manager. Product: a **terminal workspace for Claude Code
sessions** — browse, launch, arrange (tabs + splits), monitor, search,
keyboard-driven, macOS Apple Silicon. `docs/PRD.md` titles the product
"Switchboard (Rust)".

Forcing function: **`agentmux` is already taken on crates.io** (6 published
versions). Rename is not optional.

## Session Plan

| # | Phase | Technique | Status |
| --- | --- | --- | --- |
| 0 | Intake | Grounding (PRD + README) + free association | Done |
| 1 | Diverge | Affinity grouping (round 1) | Done |
| 2 | Converge | PCI on top 3 (round 1) | Done |
| 3 | Probe | crates.io sparse-index availability sweep | Done |
| 4 | Diverge | Re-seed from user-supplied terms (Shepherd, Board, Flow, Monitor, Cockpit, Term, Mux) | Done |
| 5 | Converge | Critique + rank | Done |
| 6 | Probe | Full availability sweep (crates / npm / PyPI / GitHub / domains / TM) | Done |
| 7 | Crystallize | Recommendation | Done |

---

## Round 1 — initial seeds

Grouped by metaphor:

- **Routing / switchboard:** Switchboard, Patchbay, Crossbar, Exchange,
  Relay, Junction
- **Multiplexing / weaving:** Loom, Warp, Heddle, Skein, Braid, Plait
- **Cockpit / command:** Bridge, Helm, Conn, Cockpit, Console, Foredeck
- **Conducting:** Maestro, Podium, Tutti, Downbeat, Cue
- **Harbor / mooring:** Harbor, Quay, Wharf, Mooring, Slip, Berth
- **Neologisms:** Tmuxa, Clavis, Tessera, Atrium, Veranda, Kompass

### Round-1 PCI (Switchboard / Patchbay / Loom)

| | Switchboard | Patchbay | Loom |
|---|---|---|---|
| Positives | Perfect metaphor; already in PRD | Routing metaphor without 1950s baggage; pairs with "panes" | Short, poetic, parallel-threads metaphor |
| Concerns | Long; common English word → SEO/namespace pollution | Niche (audio engineers only) | Heavy AI-namespace collision |
| Interesting | Owner already chose it | "Bay" doubles as pane | Could lean into warp/weft vocab |

### Round-1 availability probe (crates.io sparse index)

| Name | Status |
|---|---|
| agentmux | ❌ TAKEN (6 versions) — forces rename |
| switchboard | ❌ TAKEN |
| patchbay | ❌ TAKEN |
| loom | ❌ TAKEN (tokio-rs/loom — **hard blocker** for async Rust) |
| heddle | ✅ free |
| atrium | ✅ bare crate free, but `atrium-api` exists (Bluesky AT Proto) |
| quay | ❌ TAKEN |
| maestro | ❌ TAKEN |
| tessera | ❌ TAKEN |

Round-1 verdict surfaced **Heddle** (the loom part that lifts one warp
thread — i.e. selects which thread is in focus). User asked to re-run with
specific seed terms before committing.

---

## Round 2 — re-seeded with user-supplied terms

User-supplied: **Shepherd, Board, Flow, Monitor, Cockpit, Term, Mux.**

### Expansion

| Seed | Equivalents |
|---|---|
| Shepherd | Herder, Drover, Wrangler, Pastor, Sentinel, Steward, Keeper, Warden, Reeve, Bailiff, Marshal |
| Board | Dashboard, Pegboard, Plankboard, Deckboard, Slate, Foreboard, Sounding-board, Shipboard |
| Flow | Current, Eddy, Stream, Tide, Spillway, Sluice, Headrace, Confluence, Channel, Brook |
| Monitor | Watchtower, Lookout, Sentinel, Sentry, Beacon, Lighthouse, Periscope, Crow's-nest |
| Cockpit | Wheelhouse, Helm, Bridge, Cabin, Pilothouse, Foredeck, Console, Conn |
| Term | Terminus, Terma, Termon, Terminal, Boundary, Liminal, Threshold, Lintel |
| Mux | Plex, Manifold, Hub, Switch, Junction, Crossbar, Bundle, Skein |

### Mixed coinages

- Shepherd × Term → **Termherd**, Shepterm, Herdline
- Board × Mux → Plexboard, Muxboard, Crossboard
- Flow × Monitor → Flowwatch, **Tidewatch**, Spillwatch
- Cockpit × Mux → Plexpit
- Term × Cockpit → Termpit, **Termhelm**
- Shepherd × Board → Crookboard, Sheepfold, **Fold**
- Monitor × Term → Termwatch, Termsentry
- Cockpit × Board → Wheelboard, Helmboard
- Flow × Term → Confluent, Outfall, Headrace

### Round-2 availability probe (crates.io sparse index)

Free: **fold, wheelhouse, pilothouse, confluent, headrace, tidewatch,
termwatch, termherd, herdline, plexboard, muxboard, crookboard, sheepfold,
termhelm, foredeck, pegboard.**

Taken: drover, wrangler, reeve, warden, steward, marshal, sentinel, beacon,
lighthouse, conn, sluice, eddy, outfall, slate, periscope, crowsnest,
manifold, plex, skein, crossbar.

### Shortlist after critique

| Name | Pitch | Strength | Weakness |
|---|---|---|---|
| **Termherd** | Herd your terminals. | Names both the thing (terms) and the act (herd); search-clean | Slightly cute, "herd" reads informal |
| **Foredeck** | Where the work happens on deck. | Honest naval metaphor (crew managing many lines = many sessions) | Needs the metaphor unpacked |
| **Wheelhouse** | From the wheelhouse, steer every session. | Strong cockpit metaphor; idiom "in your wheelhouse" = domain mastery | 10 letters; dilutes SEO |
| Tidewatch | Watch every current. | Flow+monitor; calm, evocative | Pretty rather than precise |
| Fold | The fold — every session, gathered. | Purest shepherd metaphor | "Fold" is overloaded (verb, JS) |
| Termhelm | Take the helm of every terminal. | Term+cockpit | Coinage smell |
| Pegboard | Pin every session. | Tactile, free | Static metaphor for live processes |
| Confluent | Where the streams meet. | Free | Confluent Inc. (Kafka) owns this in dev-tools mindshare |

### Challenges (steel-manning the picks)

- **Wheelhouse vs Foredeck.** Wheelhouse = solo command (captain steers).
  Foredeck = crew working many lines. Product is closer to foredeck.
- **Termherd vs Fold.** Both shepherd-coded. Termherd is search-clean and
  self-explanatory. Fold is more poetic but loses distinctiveness.
- **Mux produced nothing strong.** Muxboard / Plexboard read like SaaS
  feature names; Termux is a famous Android app; Plex is taken.
- **Board mostly produced existing products.** Pegboard is the only
  freshwater catch and it's static.

---

## Round 3 — full availability sweep (Termherd / Foredeck / Wheelhouse)

| | **Termherd** | **Foredeck** | **Wheelhouse** |
|---|---|---|---|
| crates.io | ✅ free | ✅ free | ✅ free |
| npm | ✅ free | ✅ free | ❌ taken (Streamplace/Kubernetes CLI) |
| PyPI | ✅ free | ✅ free | ❌ taken |
| GitHub username | ✅ free | ⚠️ taken (empty org, 2018, 0 repos) | ⚠️ taken (empty user, 2010, 0 repos) |
| .com | ✅ free | ❌ registered | ❌ registered |
| .dev | ✅ free | ✅ free | ❌ registered |
| .app | ✅ free | ✅ free | ❌ registered |
| .io | ✅ free | ✅ free | ✅ free |
| .sh | ✅ free | ✅ free | ✅ free |
| Web/TM presence | none | small IT-ops co. "Foredeck Technology Inc" (foredeck.tech) — different category | multiple active products: Wheelhouse Software (Edinburgh dev shop), usewheelhouse.com (STR pricing SaaS); abandoned 1999 USPTO mark |

---

## Outcome

### Recommendation: **Termherd**

The only candidate where the namespace is clean across crates.io / npm /
PyPI / GitHub / every common TLD, with no prior commercial or trademark
footprint. The metaphor — *herding terminals* — is also the most literal
description of the product among the finalists.

### Runner-up: **Foredeck**

Credible fallback if "Termherd" reads too informal on reflection. Trade-offs:
accept `foredeck.dev` or `foredeck.io` instead of `.com`, and live next door
to a small unrelated IT-ops company. GitHub `foredeck` org is squatted but
empty (0 repos since 2018) — potentially reclaimable via GitHub's username
release policy.

### Rejected: **Wheelhouse**

Effectively burned for a developer tool. Two live commercial products use
it (one in dev tools, one in SaaS), npm/PyPI are taken, .com/.dev/.app all
registered, USPTO history. Would be the second or third "Wheelhouse"
anyone searches for.

### Action items

- [x] Pick: Termherd vs Foredeck — owner's call → **Termherd** (repo:
  `termherd`, product name: TermHerd)
- [ ] Once picked: register the relevant domain(s), GitHub org, reserve
  crate name on crates.io (publish a 0.0.0 placeholder)
- [x] Mass-rename across workspace: `Cargo.toml` package names
  (`agentmux-app` → `<name>-app`, etc.), `README.md`, `CLAUDE.md`,
  `docs/PRD.md` (currently titled "Switchboard"), `docs/ARCHITECTURE.md`
  — done 2026-06-12; "Switchboard-rs" / `~/.switchboard` replaced with
  TermHerd / `~/.termherd` in PRD + ARCHITECTURE
- [x] Decide whether to keep `Switchboard` as a doc-internal codename or
  retire it fully → **retired**; "Switchboard" now refers only to the
  upstream Electron app (`doctly/switchboard`) in `docs/background/`

---

## Session Meta-Analysis

- **Duration:** ~25 min across 3 probe rounds
- **Techniques used:** Free association → Affinity grouping → PCI on
  top 3 (×2) → Sparse-index availability probe → Full registry sweep
- **Techniques skipped:** SCAMPER (per Naming-shape rule), Six Hats
- **Adaptations:** User intercepted after round 1 to re-seed with their
  own term list — accepted, re-ran diverge/converge cycle from those
  seeds rather than continuing the AI-suggested set
- **Problem shape:** Naming (confirmed; forced by `agentmux` collision)
- **Convergence point:** Round 3 (availability sweep) — the registry
  data did the deciding, not the metaphor critique
- **What worked well:** Probing crates.io sparse index *before*
  emotional investment in any name; surfacing the `agentmux` collision
  early made the rest of the session non-optional
- **What could improve:** Round 1's PCI was done before any availability
  check — wasted effort on Switchboard/Patchbay/Loom which were all
  taken. **Lesson: in a naming session, run availability before PCI,
  not after.**
- **Session energy:** High — user actively reshaped the seeds
- **Recommendation for similar sessions:** Probe registry availability
  *between* diverge and converge phases, not after. Eliminate taken
  names before doing PCI on them.
