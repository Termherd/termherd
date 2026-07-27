+++
id = "F-session-metadata"
type = "feature"
area = ["sidebar", "sessions"]
status = "done"
target = ["Must"]
+++

Star, rename and archive sessions in an overlay beside `~/.claude`.

Star / rename / archive / custom titles for sessions (M3, moved to Must in PRD
rev. 6): a `SessionMeta` overlay in `core` persisted to
`~/.termherd/metadata.json` (never touching `~/.claude`); the browser pins
starred sessions, hides archived behind a toggle, and shows custom titles. Star
/ archive / inline rename (✎ → edit field) are all sidebar controls
