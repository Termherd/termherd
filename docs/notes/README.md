# faceto board — termherd current version

An [event-storming][es] board of termherd's **shipped state**, built with
[faceto][faceto]. It maps the real flows — discover, launch, interact,
organize, search & memory — onto the event-storming grammar, with red
hotspots marking the known gaps (#18, #19, #25, split-render, plans
editing, signing).

[es]: https://en.wikipedia.org/wiki/Event_storming
[faceto]: ../../../experiments/faceto

## The files

| File | Role | Tracked? |
| --- | --- | --- |
| `event-log.jsonl` | The **truth** — append-only event log the board replays from | yes |
| `termherd.model.json` | Genesis source the log was seeded from (readable) | yes |
| `board.svg` / `index.html` | Rendered output (regenerable) | no — gitignored |

faceto is **event-sourced**: the log is the only durable record, and the
board is a projection replayed from it. The pipeline is
`event-log.jsonl → replay → Model → SVG → HTML`.

## Install faceto

```bash
cargo install --path ../../../experiments/faceto   # puts `faceto` on PATH
```

Zero dependencies — it builds from the Rust standard library, offline.

## View the board

```bash
faceto render event-log.jsonl   # writes board.svg + index.html here
open index.html
```

## Edit it live (recommended)

```bash
faceto serve event-log.jsonl    # → http://127.0.0.1:8753
```

Click any sticky → pick a kind (`comment` / `add` / `split` / `rename` /
`drop` / `move` / `question` / `resolve`) → type a short note → **Save**.
Each save **appends an event** to `event-log.jsonl` (the click *is* the
persistence). **Reload** re-replays and shows a diff against what you were
just looking at. A later Claude session can read the new events and adjust
the board.

## Regenerate the log from the model

If you hand-edit `termherd.model.json`, re-seed the log (genesis refuses
to clobber, so remove the old one first):

```bash
rm event-log.jsonl
faceto genesis termherd.model.json   # model.json → event-log.jsonl
```

## Bound replay length (optional)

After many live edits, fold the log to a snapshot (keeps the projection,
drops per-edit history; prior log saved to `event-log.jsonl.bak`):

```bash
faceto compact event-log.jsonl
```

## The grammar

Eight lanes, coloured: `actor` · `command` · `aggregate` · `event` ·
`policy` · `readmodel` · `external` · `hotspot`. `col` is a global
left→right timeline shared across all lanes (time, not a per-lane index).
`id` is stable identity — the diff/comment join key; never renumber, only
add. A hotspot with `"resolved": true` goes quiet (grey + check) instead
of loud red.
