# Brainstorm: Interaction riche agent IA ↔ terminal via un serveur MCP termherd

| Field | Value |
| --- | --- |
| **Date** | 2026-07-13 |
| **Duration** | ~22 min |
| **Participants** | User + AI Facilitator |
| **Problem shape** | Decision under constraints × Schema/lock-in |

## Contexte / grounding

Le sujet — « étendre screenshot + enregistrement vidéo vers une interaction
riche façon browser (extension Claude / Chrome DevTools Protocol / MCP chrome /
agent-browser Vercel) » — n'est **pas une page blanche**. Deux socles existent
déjà :

- **F-capture (shippé)** est le plan *sortie / perception* : `Event::Capture`
  → `CaptureDump` (JSON état + **texte PTY visible**) + PNG ; screencast GIF
  piloté par l'horloge de présentation ; **machine à statut OSC par session**
  (`Starting/Busy/Idle/Attention/Exited`).
- **MCP est déjà la direction actée**, pas une option parmi d'autres :
  - `F-mcp-control-surface` (#90) — termherd *expose* un serveur MCP sur son
    propre contrôle (`list_options`/`set_option` + schéma, **+ orchestration :
    open session / split / focus / rename / run-in-session**). Design-first,
    **attend une passe `/feature-torture`** pour figer le scope.
  - Slice landée : `crates/mcp` (`termherd-mcp`), serveur MCP **stdio,
    read-only**, `list_options` + schéma en resource, logique pure et testée.
    **« Pour voir » — totalement reshapable.**
  - `F-mcp-ide-bridge` — l'inverse (termherd émule un IDE côté Claude). Hors
    sujet ici.

**Insight architectural clé** : termherd possède déjà une API d'interaction
complète — l'enum `Event` + `App::apply(Event) -> Vec<Effect>` (pure). Le
« moteur » est là ; il manque le **port de contrôle** (l'adaptateur qui écoute)
et le **nommage des cibles** (pas de DOM). L'analogie browser ajoute la couche
que le scope actuel de #90 sous-estime : la **perception + synchronisation**
(`snapshot`, `wait_for`) qui rend une automation « riche ».

Cible « interaction totale agent IA + terminal » = un agent *externe* orchestre
les sessions Claude *internes* : tape un prompt → **attend l'idle (statut OSC)**
→ lit la réponse (texte PTY). Browser-grade appliqué à des agents imbriqués :
terminal = « page », statut OSC = « load state », texte PTY = « DOM text ».

## Ideas — Starting Point

Les 4 protocoles-références **sont** l'espace des options, réduit par le
grounding à : *le transport est déjà tranché — MCP*. Reste le catalogue
d'outils (atomes) + le canal vers l'`App` vivant + l'échelle de rungs.

## Step 1 — Atom Inventory : catalogue d'outils MCP

Colonne « Machinerie » = existant réutilisé.

| Outil MCP | Analogie browser | Machinerie termherd | Statut |
|---|---|---|---|
| `list_options` | — | `settings.json` | ✅ landé |
| `list_sessions` / `list_tabs` | `list_pages` | `Workspace` | pur, trivial |
| `snapshot` | `read_page` | `CaptureDump` (JSON) | existe, pas outil |
| `screenshot` | `take_screenshot` | F-capture PNG | existe, pas outil |
| `read_terminal(session)` | `get_page_text` | texte PTY du `CaptureDump` | quasi-là |
| `set_option` | — | write `settings.json` | planifié #90 |
| `open_session(project,kind)` | `new_page` | `Event::Launch` | Event existe |
| `split`/`focus`/`move_tab`/`rename`/`close` | `select_page` | Events dédiés | existent |
| `type_into_terminal(session,text)` | `type_text`/`fill` | `Event::TerminalInput` | Event existe |
| `send_key`/`scroll`/`select`/`copy` | `press_key` | Events | existent |
| `wait_for_status(session, Idle\|Attention)` | `wait_for` | **statut OSC** | ⭐ pivot |
| `wait_for_text(session, pattern)` | `wait_for` (texte) | texte PTY + poll | à concevoir |
| `capture`/`toggle_record` | recording | Events | existent |

~80 % = wrapping mince d'`Event`/`CaptureDump`/statut OSC déjà construits. Le
neuf réel : (1) canal live vers le GUI, (2) nommage de cible stable, (3)
`wait_for_*`, (4) sécurité des writes/exec.

## Step 2 — Constraint Mapping (le crux IPC)

| Contrainte | Ce qu'elle force |
|---|---|
| core pur | le handler construit des `Event` ; transport = adaptateur |
| `apply` single-threaded (UI thread) | toute requête **traverse la boucle iced** : req → `Message` → `apply` → résultat → réponse. **Brique à valider.** |
| single-instance flock | un GUI vivant → un serveur |
| leçon Q7 (`openDiff` hang) | **chaque aller-retour borné par timeout** |
| MIT / no-FFI / dép-frugal | réutiliser `notify`/`serde_json` avant d'ajouter un serveur lourd |
| sécurité writes/exec | loopback-only + token par session |

**Fait décisif** : le client MCP de Claude Code parle **`stdio` OU `http/sse`,
pas de socket brut**. Donc le seul chemin *sans frontière de process* est un
serveur **http/sse in-process** hébergé par le GUI (Claude se connecte à une
`url`, le handler tient `core::App` directement — zéro sérialisation, zéro
protocole de corrélation). Le chemin stdio (ou file-drop) ré-introduit
forcément la frontière de process.

**Prérequis partagé, pas encore construit** : injecter un `mcpServers` dans la
session Claude au lancement (`--mcp-config` / `.mcp.json`). Aujourd'hui
`Launch::Claude` ne porte que `{ resume }`. Adjacent à `F-mcp-launch-flags`.

## Décision (validée)

**Transport cible = serveur MCP hébergé in-process dans le GUI, `http/sse`
loopback + token par session, injecté dans le `mcpServers` de chaque session au
lancement.** Seul chemin sans frontière de process ; matche le cadrage #90
(« termherd est le serveur »). **`file-drop` (réutilise `notify`) = fallback
documenté** si l'expérience invalide le round-trip. On ne construit **pas** deux
transports.

La slice stdio du collègue est reshapée : on garde la **moitié pure** (dispatch
JSON-RPC + schéma), on **jette la boucle stdio**, re-logée derrière
l'adaptateur http/sse.

### ⚗️ Expérience de validation/invalidation (le gate)

Spike : **un seul outil read (`list_sessions`) en round-trip complet** — client
externe → http/sse → `Message` iced → `App::apply` (UI thread) → état relu →
réponse, **borné par timeout**.

- ✅ Vert (round-trip propre, borné, cross-plateforme, pas de deadlock) → le
  catalogue n'est plus que « des outils en plus » ; on déroule les rungs.
- ❌ Rouge (deadlock / ordering / timeout ingérable / dép HTTP trop lourde) →
  repli **file-drop** pour le contrôle grossier ; on diffère la couche synchro.

Risque porteur = « traverser la boucle iced avec un req/rép borné » —
transport-agnostique.

## Step 3 — Le catalogue progressif en rungs

- **Rung 0 — ✅ landé (à reshaper)** : `list_options`. Garder le pur, jeter
  stdio.
- **Rung 1 — prérequis** : injection `mcpServers` au launch (`--mcp-config`) +
  `set_option`. *Débloque tout.*
- **Rung 2 — ⚗️ le spike** : http/sse in-process + `list_sessions` / `snapshot`
  / `screenshot` (perception read-only live). **Le gate.**
- **Rung 3 — action** : `open_session` / `split` / `focus` / `rename` /
  `close` (le scope littéral #90).
- **Rung 4 — synchro** : `wait_for_status` (statut OSC déjà là) +
  `read_terminal`. La brique « riche ».
- **Rung 5 — agent-drives-agent** : `type_into_terminal` + boucle
  prompt→wait→read, **derrière opt-in** (surface de risque max).

## Step 4 — Impact / Effort

| | Effort faible | Effort élevé |
|---|---|---|
| **Impact fort** | `list_sessions` · `snapshot` · `set_option` | transport http/sse · injection `mcpServers` · `wait_for_status` · `type_into_terminal`+boucle |
| **Impact faible** | `screenshot` · `read_terminal` · `rename`/`move_tab`/`close`/`split`/`focus` | ~~`wait_for_text`~~ · ~~DSL sélecteurs~~ → différer |

Valeur + risque concentrés sur **3 briques** : transport, `wait_for_status`,
injection. Coupés (YAGNI) : `wait_for_text` (poll racé), DSL de sélecteurs (un
id stable suffit).

## Step 5 — Pre-mortem `day-1-blocker`

1. **⛔ Id de session instable** (Q6 `realSessionId` re-key) — l'arg `session`
   est un contrat ; un re-key en vol vise une session morte. → **handle
   pane/session interne stable, découplé du re-key**. *Blocker n°1, avant tout
   write.*
2. **⛔ Round-trip pendu** (Q7 rejoué) — `tokio::timeout` sur chaque outil ;
   *apply-and-read*, jamais « attendre la complétion d'un Effect ».
3. **⛔ Boucle agent-drives-agent** — opt-in + cap max-wait + timeout
   systématique sur `wait_for_status`.
4. **Token/port** — port éphémère, token par session, jamais loggé, injecté
   par config pas argv.
5. **Injection qui clobber** — merge (pas overwrite) d'un `.mcp.json`
   utilisateur ; seulement `Launch::Claude`.
6. **Screenshot en arrière-plan** — best-effort ; `snapshot` texte = chemin
   fiable.

---

## Outcome

### Selected Ideas / Decisions

1. **Le transport est MCP, hébergé in-process (http/sse loopback + token)** —
   seul chemin sans frontière de process pour atteindre l'`App` vivant ; matche
   #90. File-drop = fallback, pas cible.
2. **Reshape la slice du collègue** — garder le pur (JSON-RPC + schéma), jeter
   la boucle stdio.
3. **Échelle de 6 rungs, chacun shippable** — 0 landé, 1 prérequis (injection +
   writes), 2 = le spike/gate, 3 action, 4 synchro, 5 agent-drives-agent
   (opt-in).
4. **Un spike invalidant en Rung 2** — `list_sessions` round-trip borné ; vert
   ⇒ le reste est « des outils en plus », rouge ⇒ file-drop + diffère la synchro.
5. **3 blockers à traiter d'entrée** — id de session stable, timeout par outil,
   opt-in sur agent-drives-agent.

### Action Items

- [ ] Lancer `/feature-torture` sur `F-mcp-control-surface` (#90) en injectant
      ce catalogue + les rungs + le pre-mortem — la passe que #90 attend déjà.
- [ ] Rédiger le spike Rung 2 (`list_sessions` in-process http/sse,
      timeout-borné) comme expérience de validation/invalidation.
- [ ] Filer l'issue prérequis « injection `mcpServers` au launch » (Rung 1),
      cross-liée à `F-mcp-launch-flags`.
- [ ] Mettre à jour l'entrée `F-mcp-control-surface` du ROADMAP avec l'échelle
      de rungs + le lien vers ce brainstorm (rider dans la PR de la première
      slice, pas une PR doc standalone).

---

## Session Meta-Analysis

- **Duration:** ~22 min
- **Techniques used:** Grounding (2×) → Atom Inventory → Constraint Mapping →
  Impact/Effort → Pre-mortem (`day-1-blocker`)
- **Techniques skipped:** SCAMPER (atomes d'un artefact structuré → hallucine
  des variantes) ; Six Hats (non demandé)
- **Adaptations made:** reshape majeur en INTAKE — le grounding a révélé que MCP
  était déjà acté (#90 + slice landée), transformant « choisir un protocole
  parmi 5 » en « tracer les rungs d'un MCP déjà projeté ». Le fait « client MCP
  = stdio|http, pas socket » a binarisé le crux transport.
- **Problem shape:** Decision under constraints → confirmé, teinté Schema/lock-in
  (le catalogue d'outils = atomes).
- **Convergence point:** Step 2 (Constraint Mapping) — le fait décisif
  transport a tranché le crux et cascadé sur les rungs.
- **What worked well:** grounder AVANT de générer a évité un brainstorm entier
  sur des options déjà tranchées par le repo ; le mapping browser-primitive →
  machinerie termherd existante a montré que 80 % est du wrapping.
- **What could improve:** la 1ʳᵉ passe de questions (AskUserQuestion) était
  prématurée — poser le crux avant d'avoir grounté le MCP existant.
- **Session energy:** high — l'utilisateur a redirigé deux fois avec du contexte
  décisif (MCP déjà projeté ; slice reshapable).
- **Recommendation for similar sessions:** pour tout « automatiser/étendre X
  dans l'app Y », grep les built-ins ET le roadmap/PRD avant de proposer des
  architectures — la moitié des « options » sont souvent déjà tranchées.
