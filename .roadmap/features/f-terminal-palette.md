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

Since then the settings panel made both keys live, and the catalogue widened
to every iced chrome theme (22) with 14 terminal schemes taken from
alacritty-theme (Apache-2.0) beside the four originals, each named like its
chrome theme. One table in `pty::schemes` now drives both the picker and the
lookup. Kanagawa Lotus and Ferra stay chrome-only: no terminal palette is
published for them. The chrome marks (status dots, `/color`) shade per
surface to hold the WCAG 3:1 floor on every theme, instead of a fixed
dark/light step.
