+++
id = "F-terminal-palette"
type = "feature"
area = ["terminal"]
status = "done"
target = ["Could"]
+++

Configurable terminal colours, by preset or by explicit field.

Configurable terminal colours (#181, shipped in #183; tortured 👍,
feature-torture `F-terminal-palette.md`): an optional `terminal.colors` block
in `settings.json` — `foreground`, `background`, `cursor` and the 16-colour
ANSI `palette`, plus a `scheme` picking a built-in preset
(`solarized-dark`/`-light`, `gruvbox-dark`/`-light`) that explicit fields
override. A `Palette` is injected into `PtyManager::new` like the shell profile
— colours keep resolving in the `pty` adapter, `core` never sees RGB, and
`Screen` carries `default_bg`/`cursor_color` so the canvas dropped its
duplicated constants. Wide-parse per field: a bad value warns and degrades
alone. Restart-to-apply; the MCP catalog exposes the five keys. Dims stay a
fixed hand-tuned table (legibility guards). Deliberately out: selection colour
(app affordance), live reload (waits for the in-app settings panel). Verified
end-to-end via F-capture on a real session (Solarized Light)

Since then (#381): both keys apply live; all 22 iced chrome themes, 18
same-name schemes (14 from alacritty-theme, Apache-2.0) in one `pty::schemes`
table; chrome marks shaded per theme to hold 3:1. Kanagawa Lotus and Ferra
have no published palette. The paragraph above describes #183 as shipped;
its four presets, "restart-to-apply" and "live reload out" are superseded by
this entry.

Open (2026-10-29): the status dot on the active tab chip is shaded against the
window background, not the chip's fill, and can sit below 3:1 (#382). Not
ticketed: dropping the fixed light-theme darkening for a pure contrast search
would shift the stock light themes' colours, a visual call nobody has made.
Licences for the imported schemes: `THIRD_PARTY_NOTICES.md`.
