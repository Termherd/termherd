# The sidebar: browse, search, star

The sidebar is the session browser. It lists every Claude session TermHerd
found by walking `~/.claude/projects`, grouped by project, and it refreshes
live as the filesystem changes — a session started elsewhere appears without a
restart.

Toggle it with <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>B</kbd>.

```text
┌──────────────────────────┐
│ Search…                  │  ← Cmd/Ctrl+F focuses this
│ ☐ Titles only            │
│ ☐ Show archived          │
├──────────────────────────┤
│ ★ Favorites              │
│   my-app · fix the race  │
├──────────────────────────┤
│ ▾ my-app          $  🤖  │  ← launch a shell / a fresh Claude session
│     fix the race  ★ ⊟ ✎  │
│     add the cache ★ ⊟ ✎  │
│     … 3 more             │
│ ▸ other-repo      $  🤖  │
├──────────────────────────┤
│ Plans & memory           │
│   CLAUDE.md (global)     │
│   my-app/CLAUDE.md       │
│   plan-2026-08-01.md     │
└──────────────────────────┘
```

## Opening a session

Click a project to expand it, then a session to resume it in a new tab.

Beside each project row are two launch buttons: **`$`** opens a plain shell in
that project's directory, **🤖** starts a fresh Claude session there. The same
two actions are on the keyboard as <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>T</kbd>
and <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>T</kbd>, which use the
*focused* session's
directory instead.

Hovering a session shows a card with its fuller description — relative last
activity and message count (`3h ago · 214 messages`). The same card is what a
tab shows on hover.

## Search

<kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>F</kbd> focuses the search box. Search runs
in memory over the scan's digests — titles, slugs, first prompts and indexed
transcript text.

- A **content hit** renders the matched line in muted text under the session
  row, windowed around the match and clipped to the sidebar width, so you can
  tell *why* a row matched.
- **Titles only** narrows the search to titles and slugs.
- **Show archived** brings archived sessions back into the results.

Search is in-memory by design at this stage: the SQLite + FTS5 digest cache
(`F-store-cache`) is an optimisation on the roadmap, not shipped. On a very
large `~/.claude` history the first scan is the slow part, not the query.

## Stars, renames, archives

Three buttons on each session row:

| Button | Does |
| --- | --- |
| ★ | star / unstar — starred sessions collect under **★ Favorites** at the top |
| ⊟ | archive (with a confirmation) — hidden unless **Show archived** is on |
| ✎ | rename — an inline field; <kbd>Escape</kbd> abandons the edit |

All three are an **overlay**: they are written to
`~/.termherd/metadata.json`, never under `~/.claude`. Nothing you do in the
sidebar changes what the Claude CLI sees.

## Density

Long projects fold: past `sidebar.session_limit` sessions (default 5) the tail
collapses behind a `… N more` expander, and `show less` folds it back. Set the
limit to `0` to always show every session —
[`settings.json`](../reference/settings.md).

## Plans & memory

The bottom section lists Claude's plan files (`~/.claude/plans/*.md`) and the
memory files: the global `~/.claude/CLAUDE.md` and each project's `CLAUDE.md`.
Clicking one loads it off-thread and opens it in the main pane, replacing the
terminal view until you close it.

The doc pane can **edit** — `💾 save`, with a `• modified` marker while there
are unsaved changes and `saved` after a write. Two ways out, and they behave
identically: the `✕ close` button, or <kbd>Escape</kbd>. Both **discard unsaved
edits without asking** — a known rough edge, tracked as
[#248](https://github.com/Termherd/termherd/issues/248).

## When nothing is open

With no session open, the main pane shows a welcome card: how many sessions in
how many projects were found, and the two ways to start — the `$` / 🤖 buttons
beside a project, or a click on a session to resume it.
