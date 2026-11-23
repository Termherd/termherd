# termherd design system

termherd's chrome is drawn by iced, not CSS. Most of its look comes from the
iced theme the user picks. Its own design choices are the few values the view
hardcodes on top of that palette. This page records those values and the
reason for each, so a change to the interface can be judged against them.
Who the interface is for, and what it must never become, is in
[`PRODUCT.md`](PRODUCT.md).

The code is the source of truth, and the build does not read this page: a
change to `crates/app/src/shell` that moves a value updates it in the same
pull request.

## Principles

1. **The theme owns the surfaces.** Backgrounds, text and the accent come from
   `iced::theme::palette::Extended`. The view picks a palette tier
   (`background.base`, `weak`, `strong`, `primary`) and never a hex value.
2. **A mark must read, whatever the theme.** Status dots and session colours
   are pushed toward black or white until they reach 3:1 contrast, the WCAG
   floor for non-text components, on every surface they sit on.
3. **Colour encodes state, never decoration.** Status is the only thing a dot
   shows. A session colour marks the tab outline and the sidebar bar, and
   leaves the title text in the theme's own colour.
4. **Dense and keyboard-first.** Text runs from 11 to 15 px, and spacing
   stays at 8 px or less inside a row. The terminal grid gets the pixels.

## Colour

### Roles from the theme

| Role | Palette tier | Used by |
| --- | --- | --- |
| Surface | `background.base` | window, sidebar, mark backdrop |
| Raised surface | `background.weak` | mark backdrop, card border |
| Card | `background.strong` | hover card, swatch border, pane border |
| Accent | `primary.base` | active tab chip fill |
| Accent strong | `primary.strong` | focused pane border, drag caret |

Muted text is mixed from these tiers, not taken from a grey: sidebar snippets
are the text colour 40 % of the way to the surface, and card secondary text is
35 % of the way.

### Status hues (FR8)

Tuned for a dark surface. On a light theme each hue first moves 40 % toward
black, then shades further until it clears 3:1
(`style.rs`, `on_surface`). On the active chip it shades toward whichever
of black and white contrasts more with the fill (`on_fill`).

| Status | Hue | On white | On black |
| --- | --- | --- | --- |
| Starting | `#8c8c99` | 3.31 | 6.35 |
| Busy | `#f2b233` | 1.87 | 11.24 |
| Idle | `#4ccc66` | 2.07 | 10.14 |
| Attention | `#f25959` | 3.30 | 6.37 |
| Exited | `#808080` | 3.98 | 5.28 |

The ratios are for the raw hue, before any shading. They show why the shading
exists. Source: a contrast script over the `Color::from_rgb` values in
`status_color`.

### Session colours

The eight `/color` names Claude Code accepts, shaded like the status hues.
`default` paints nothing and draws as an empty frame in the picker.

| Name | Hue | Name | Hue |
| --- | --- | --- | --- |
| red | `#ff6b66` | purple | `#bd85ff` |
| blue | `#6b9eff` | orange | `#fa9426` |
| green | `#5cc761` | pink | `#f273cc` |
| yellow | `#c7b20f` | cyan | `#3dccdb` |

Tests in `style.rs` hold three invariants. Every hue reads on every theme's
surfaces. No two hues land closer than 0.2 in summed RGB distance. The six
stock themes keep the exact shades they had before shading existed.

### Terminal schemes

The terminal grid has its own palette (`crates/pty/src/schemes.rs`). With no
scheme set, it follows the chrome theme: the theme's own scheme, or a
neighbouring one of the same lightness (`ThemeChoice::terminal_scheme` in
`settings.rs`), so a light chrome never frames a dark grid. The settings
panel can still pin a scheme apart from the theme.

## Typography

Two families: the iced default for chrome, `Font::MONOSPACE` for the terminal
and the document editor.

Every chrome text takes one of four steps, named in `view/style.rs`:

| Step | px | Used for |
| --- | --- | --- |
| `CAPTION` | 11 | search snippet, shortcut hint, tooltip, save note, menu heading |
| `BODY` | 13 | sidebar rows, tab titles, buttons, prompts, fields |
| `HEADING` | 15 | project names, panel titles, the chrome's icon buttons |
| `DISPLAY` | 40 | the empty-state wordmark |

Each step is at least 1.15 times the one below, a const assertion beside the
constants: closer than that, two sizes read as one, not as hierarchy. A test
reads every size the view passes and fails on anything off the scale. Two
marks sit outside it by name: the status dot (`STATUS_DOT_SIZE`, 9) and the
search toggles' box (`CHECKBOX_SIZE`, 14).

The terminal defaults to 14 px. Its cell is 0.6 × 1.286 of the font size,
which gives 8.4 × 18 px at the default (`terminal/mod.rs`).

## Spacing

| Token | px |
| --- | --- |
| 0 | 0 |
| px | 1 |
| xxs | 2 |
| xs | 4 |
| sm | 6 |
| md | 8 |
| lg | 12 |
| xl | 16 |

Spacing is still written as literals in each call. `sm` sits off the 4 px
grid and is the most used step: a choice, kept.

## Shape

| Token | px | Used by |
| --- | --- | --- |
| none | 0 | sidebar row |
| hairline | 1 | sidebar colour bar |
| sm | 2 | swatch, insertion caret |
| md | 4 | tab chip, pane border |
| lg | 6 | hover card |

Borders are 1 px (card, swatch) or 2 px for a session-colour mark and a pane
border (1 px when the pane is not focused).

## Layout

| Constant | px | Source |
| --- | --- | --- |
| Sidebar width | 300 | `geometry.rs` `SIDEBAR_W` |
| Sidebar handle | 28 | `geometry.rs` `HANDLE_W` |
| Split handle | 8 | `geometry.rs` `SPLIT_HANDLE` |
| Pane padding | 2 | `geometry.rs` `PANE_PAD` |
| Pane border | 2 | `geometry.rs` `PANE_BORDER` |
| Hover card width | 240 | `view/tabs.rs` |
