# The sidebar: browse, search, star

The sidebar is the session browser. It lists every Claude session TermHerd
found by walking `~/.claude/projects`, grouped by project — together with the
repositories you added by hand, which need no session to appear. It refreshes
live as the filesystem changes, so a session started elsewhere shows up without
a restart.

Toggle it with <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>B</kbd>.

```text
┌──────────────────────────┐
│ ◀ Hide       + Add a repo  ⚙ │  ← add a folder · open the settings panel
│ Search…                  │  ← Cmd/Ctrl+F focuses this
│ ☐ Titles only            │
│ ☐ Show archived          │
├──────────────────────────┤
│ ★ Favorites              │
│   my-app · fix the race  │
├──────────────────────────┤
│   new-repo     ❯  ✳   ✕  │  ← added by hand, no sessions yet
│     No sessions yet …    │
│ ▾ my-app          ❯  ✳   │  ← launch a shell / a fresh Claude session
│     fix the race  ★ ⊟ ✎  │
│     add the cache ★ ⊟ ✎  │
│     … 3 more             │
│ ▸ other-repo      ❯  ✳   │
├──────────────────────────┤
│ Plans & memory           │
│   CLAUDE.md (global)     │
│   my-app/CLAUDE.md       │
│   plan-2026-08-01.md     │
└──────────────────────────┘
```

## Opening a session

Click a project to expand it, then a session to resume it in a new tab.

Beside each project row are two launch buttons: **`❯`** opens a plain shell in
that project's directory, **`✳`** starts a fresh Claude session there — under a
session id TermHerd picks (`claude --session-id <uuid>`), so once the session
shows up in the list, clicking it brings its tab forward instead of resuming
a second copy. The same
two actions are on the keyboard as <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>T</kbd>
and <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>T</kbd>, which use the
*focused* session's
directory instead.

## Adding a repository

A project only appears once it has a Claude session, so a repository you have
never opened Claude in is invisible — including the one you are about to start
work in. **+ Add a repo** puts it there anyway: pick a folder, or **drop one on
the window**. Both do the same thing.

Drop a **folder**, not a file: a dropped file is ignored, so dragging one onto
a terminal never quietly adds its directory.

The row that appears carries the same `❯` and `✳` buttons as any other, and says
`No sessions yet` until it has one. When it does, it becomes an ordinary
project row — there is no second entry.

An added repository sorts to the top of the list until its first session, then
takes its place by recency like everything else. A **✕** on its row removes it;
the button only appears on rows you added, since a discovered project has no
declaration to drop. Removing a repository that has since gained sessions drops
only the addition — the project stays, because the scan still finds it.

What you pick is not always what is stored, but it is filed by **exactly the
rule the scan uses** for a session's working directory — that agreement is what
keeps one repository on one row. A git worktree collapses onto its main
checkout; a file becomes the folder holding it; everything else is stored as
given, symlinks and all.

One consequence is worth knowing: a **subdirectory is not climbed**. Add
`~/dev/app/crates/core` and you get a row for that subdirectory, not for
`~/dev/app` — because a Claude session started there is filed the same way.
Add the directory you actually want the row for.

Additions live in `~/.termherd/metadata.json`, beside stars and renames.
Nothing is written under `~/.claude`.

Hovering a session shows a card with its fuller description — relative last
activity and message count (`3h ago · 214 messages`), the model and reasoning
effort of its last reply (`Model: claude-opus-5-5  ·  medium effort`) and the
Claude Code version that wrote it (`Claude Code 2.1.294`). A line is left out
when the transcript does not record it: older Claude Code versions write no
effort. A tab shows the same card on hover, plus the live facts described in
[Tabs and splits](tabs-and-splits.md).

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
how many projects were found, and the two ways to start — the `❯` / `✳` buttons
beside a project, or a click on a session to resume it.
