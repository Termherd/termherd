//! The terminal grid ↔ [`Screen`] boundary: the rendered-cell types, the
//! colour palette and its resolution to RGB, and the snapshot/selection code
//! that turns an `alacritty_terminal` grid into the GUI-facing [`Screen`].
//! Depends on `alacritty_terminal` (and the neutral `SelectOp` type) only, so
//! the shell needs no terminal knowledge.

use alacritty_terminal::Term;
use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::{Flags, Hyperlink};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use termherd_core::{MouseReporting, PointerEvent, SelectOp, SelectSide, pointer_select};

use crate::mode::mouse_reporting;

/// A snapshot of the visible terminal grid handed to the GUI for rendering.
/// Colours are resolved to RGB here so the shell needs no terminal knowledge.
#[derive(Debug, Clone)]
pub struct Screen {
    pub cols: u16,
    pub rows: u16,
    /// Visible rows, top to bottom; each is exactly `cols` cells wide.
    pub lines: Vec<Vec<ScreenCell>>,
    /// Cursor position as `(col, row)` in visible coordinates, if shown.
    pub cursor: Option<(u16, u16)>,
    /// True while the viewport is scrolled up into scrollback history.
    pub scrolled: bool,
    /// How many lines the viewport is scrolled up into scrollback history
    /// (0 at the bottom / live tail). Lets the GUI anchor a selection to an
    /// absolute scrollback line so the highlight follows the text through
    /// scroll, rather than floating over whatever now occupies the row.
    pub display_offset: usize,
    /// True when the application has enabled bracketed paste (DECSET 2004), so
    /// the shell wraps a paste in `ESC[200~`…`ESC[201~` and a multi-line paste
    /// lands as one block instead of submitting line by line (FR4).
    pub bracketed_paste: bool,
    /// The mouse reporting the application has switched on, if any — when set
    /// the mouse is the child's, so a pointer event over the grid is forwarded
    /// rather than driving the terminal's own selection.
    pub mouse_reporting: Option<MouseReporting>,
    /// The highlighted selection as inclusive per-row column spans `(row, c0, c1)`
    /// in visible coordinates, one per on-screen row the selection covers, empty
    /// when nothing is selected. Derived from the terminal's own selection, which
    /// the emulator rotates on every grid scroll — so the highlight follows the
    /// text through both scrollback and application-driven (alt-screen) scroll.
    pub selection: Vec<(u16, u16, u16)>,
    /// Every OSC 8 hyperlink on screen, as the runs of cells it covers. The
    /// target rides beside the grid rather than in each cell so a link whose
    /// label hides its URL (`#76` over an issue URL) still resolves, and
    /// [`ScreenCell`] stays a plain `Copy` value.
    pub hyperlinks: Vec<HyperlinkSpan>,
    /// The palette's default background — what the GUI paints behind the grid
    /// (and skips repainting per cell). Carried here so the shell needs no
    /// palette knowledge.
    pub default_bg: [u8; 3],
    /// The palette's cursor colour, for the GUI's cursor block.
    pub cursor_color: [u8; 3],
}

/// One rendered grid cell: a character and its resolved colours.
#[derive(Debug, Clone, Copy)]
pub struct ScreenCell {
    pub c: char,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub bold: bool,
}

/// One contiguous run of cells sharing an OSC 8 hyperlink, in visible
/// coordinates: columns `start..end` (exclusive) of `row`, and the URI they
/// point at. The same link printed in two places is two spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperlinkSpan {
    pub row: u16,
    pub start: u16,
    pub end: u16,
    pub uri: String,
}

/// Folds the grid's cells into [`HyperlinkSpan`]s: a cell carrying the same
/// hyperlink as the cell before it extends the open span, anything else closes
/// it. Cells must arrive in row-major order — the display iterator's — so a
/// span's end column alone tells whether the next cell continues it: a new
/// row starts at column 0, which no open span ends at.
#[derive(Default)]
struct HyperlinkRuns {
    spans: Vec<HyperlinkSpan>,
    open: Option<(HyperlinkSpan, Hyperlink)>,
}

impl HyperlinkRuns {
    fn push(&mut self, row: u16, col: u16, link: Option<Hyperlink>) {
        if let Some((span, current)) = &mut self.open
            && let Some(link) = &link
            && span.end == col
            && current == link
        {
            span.end = col + 1;
            return;
        }
        self.close();
        if let Some(link) = link {
            let span = HyperlinkSpan {
                row,
                start: col,
                end: col + 1,
                uri: link.uri().to_owned(),
            };
            self.open = Some((span, link));
        }
    }

    fn close(&mut self) {
        if let Some((span, _)) = self.open.take() {
            self.spans.push(span);
        }
    }

    fn finish(mut self) -> Vec<HyperlinkSpan> {
        self.close();
        self.spans
    }
}

/// The terminal colour scheme: the default foreground/background, the cursor
/// block, and the 16 ANSI colours. Built from `settings.json` in the
/// composition root and injected into `PtyManager::new` (FR10); [`Default`]
/// is the built-in scheme. The dim named colours stay a fixed, hand-tuned
/// table — they are legibility guards, not part of the configurable 16.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    /// Default text colour when a cell uses the terminal's foreground.
    pub foreground: [u8; 3],
    /// Default background — also what the GUI paints behind the grid.
    pub background: [u8; 3],
    /// The cursor block colour.
    pub cursor: [u8; 3],
    /// The 16 ANSI colours, indices 0–15.
    pub ansi: [[u8; 3]; 16],
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            foreground: DEFAULT_FG,
            background: DEFAULT_BG,
            cursor: DEFAULT_FG,
            ansi: ANSI16,
        }
    }
}

impl Palette {
    /// An empty cell in this palette's default colours.
    const fn blank_cell(&self) -> ScreenCell {
        ScreenCell {
            c: ' ',
            fg: self.foreground,
            bg: self.background,
            bold: false,
        }
    }

    /// A built-in scheme by its settings name (`terminal.colors.scheme`), or
    /// `None` when unknown. The unnamed built-in default is
    /// [`Palette::default`]. Values from the published Solarized (Ethan
    /// Schoonover) and Gruvbox (Pavel Pertsev) specifications, both MIT.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        let (foreground, background, ansi) = match name {
            "solarized-dark" => (hex(0x839496), hex(0x002b36), SOLARIZED_ANSI),
            "solarized-light" => (hex(0x657b83), hex(0xfdf6e3), SOLARIZED_ANSI),
            "gruvbox-dark" => (hex(0xebdbb2), hex(0x282828), GRUVBOX_DARK_ANSI),
            "gruvbox-light" => (hex(0x3c3836), hex(0xfbf1c7), GRUVBOX_LIGHT_ANSI),
            _ => return None,
        };
        Some(Self {
            foreground,
            background,
            cursor: foreground,
            ansi,
        })
    }
}

/// Split a `0xrrggbb` literal into RGB — lets the scheme tables read as the
/// hex values their specifications publish.
const fn hex(c: u32) -> [u8; 3] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8]
}

/// Solarized accents (shared by the dark and light variants, which differ
/// only in the base tones the fg/bg pick).
const SOLARIZED_ANSI: [[u8; 3]; 16] = [
    hex(0x073642), // base02
    hex(0xdc322f), // red
    hex(0x859900), // green
    hex(0xb58900), // yellow
    hex(0x268bd2), // blue
    hex(0xd33682), // magenta
    hex(0x2aa198), // cyan
    hex(0xeee8d5), // base2
    hex(0x002b36), // base03
    hex(0xcb4b16), // orange
    hex(0x586e75), // base01
    hex(0x657b83), // base00
    hex(0x839496), // base0
    hex(0x6c71c4), // violet
    hex(0x93a1a1), // base1
    hex(0xfdf6e3), // base3
];

const GRUVBOX_DARK_ANSI: [[u8; 3]; 16] = [
    hex(0x282828),
    hex(0xcc241d),
    hex(0x98971a),
    hex(0xd79921),
    hex(0x458588),
    hex(0xb16286),
    hex(0x689d6a),
    hex(0xa89984),
    hex(0x928374),
    hex(0xfb4934),
    hex(0xb8bb26),
    hex(0xfabd2f),
    hex(0x83a598),
    hex(0xd3869b),
    hex(0x8ec07c),
    hex(0xebdbb2),
];

const GRUVBOX_LIGHT_ANSI: [[u8; 3]; 16] = [
    hex(0xfbf1c7),
    hex(0xcc241d),
    hex(0x98971a),
    hex(0xd79921),
    hex(0x458588),
    hex(0xb16286),
    hex(0x689d6a),
    hex(0x7c6f64),
    hex(0x928374),
    hex(0x9d0006),
    hex(0x79740e),
    hex(0xb57614),
    hex(0x076678),
    hex(0x8f3f71),
    hex(0x427b58),
    hex(0x3c3836),
];

impl Screen {
    /// An empty `cols`×`rows` screen in the default palette at the live tail:
    /// nothing scrolled, selected, linked or negotiated. The base a fixture
    /// overrides one field of, so a new `Screen` field costs one edit here
    /// rather than one per hand-rolled literal.
    #[must_use]
    pub fn blank(cols: u16, rows: u16) -> Self {
        let palette = Palette::default();
        Self {
            cols,
            rows,
            lines: vec![vec![palette.blank_cell(); usize::from(cols)]; usize::from(rows)],
            cursor: None,
            scrolled: false,
            display_offset: 0,
            bracketed_paste: false,
            mouse_reporting: None,
            selection: Vec::new(),
            hyperlinks: Vec::new(),
            default_bg: palette.background,
            cursor_color: palette.cursor,
        }
    }

    /// Flatten the visible grid to plain text (trailing blanks trimmed) — for
    /// logging and tests.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = String::with_capacity(self.lines.len() * (self.cols as usize + 1));
        for line in &self.lines {
            let row: String = line.iter().map(|cell| cell.c).collect();
            out.push_str(row.trim_end());
            out.push('\n');
        }
        out.trim_end_matches('\n').to_string()
    }
}

/// Default foreground/background when a cell uses the terminal's defaults.
const DEFAULT_FG: [u8; 3] = [0xd0, 0xd0, 0xd0];
const DEFAULT_BG: [u8; 3] = [0x11, 0x13, 0x18];

/// The 16 ANSI colours (classic VGA palette), indices 0–15.
const ANSI16: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00],
    [0xcc, 0x33, 0x33],
    [0x33, 0xcc, 0x33],
    [0xcc, 0xcc, 0x33],
    [0x33, 0x66, 0xcc],
    [0xcc, 0x33, 0xcc],
    [0x33, 0xcc, 0xcc],
    [0xcc, 0xcc, 0xcc],
    [0x66, 0x66, 0x66],
    [0xff, 0x66, 0x66],
    [0x66, 0xff, 0x66],
    [0xff, 0xff, 0x66],
    [0x66, 0x99, 0xff],
    [0xff, 0x66, 0xff],
    [0x66, 0xff, 0xff],
    [0xff, 0xff, 0xff],
];

/// Resolve an xterm 256-colour index to RGB (16 ANSI + 6×6×6 cube + ramp).
pub(crate) fn indexed_rgb(i: u8, palette: &Palette) -> [u8; 3] {
    match i {
        0..=15 => palette.ansi[i as usize],
        16..=231 => {
            let n = i - 16;
            let levels = [0u8, 95, 135, 175, 215, 255];
            [
                levels[(n / 36) as usize],
                levels[((n / 6) % 6) as usize],
                levels[(n % 6) as usize],
            ]
        }
        232..=255 => {
            let v = 8 + 10 * (i - 232);
            [v, v, v]
        }
    }
}

/// Resolve a named colour to RGB against the palette. The dim variants keep a
/// fixed, hand-tuned table (see [`Palette`]).
fn named_rgb(named: NamedColor, palette: &Palette) -> [u8; 3] {
    use NamedColor::*;
    match named {
        Black => palette.ansi[0],
        Red => palette.ansi[1],
        Green => palette.ansi[2],
        Yellow => palette.ansi[3],
        Blue => palette.ansi[4],
        Magenta => palette.ansi[5],
        Cyan => palette.ansi[6],
        White => palette.ansi[7],
        BrightBlack => palette.ansi[8],
        BrightRed => palette.ansi[9],
        BrightGreen => palette.ansi[10],
        BrightYellow => palette.ansi[11],
        BrightBlue => palette.ansi[12],
        BrightMagenta => palette.ansi[13],
        BrightCyan => palette.ansi[14],
        BrightWhite => palette.ansi[15],
        DimBlack => palette.ansi[0],
        DimRed => [0x88, 0x22, 0x22],
        DimGreen => [0x22, 0x88, 0x22],
        DimYellow => [0x88, 0x88, 0x22],
        DimBlue => [0x22, 0x44, 0x88],
        DimMagenta => [0x88, 0x22, 0x88],
        DimCyan => [0x22, 0x88, 0x88],
        DimWhite => [0x88, 0x88, 0x88],
        Foreground | BrightForeground => palette.foreground,
        DimForeground => [0x99, 0x99, 0x99],
        Background => palette.background,
        Cursor => palette.cursor,
    }
}

/// Darken a colour for the faint/dim attribute (SGR 2): ~60% intensity, so
/// dim text reads as grey rather than the near-white it had when the flag was
/// ignored.
fn dim([r, g, b]: [u8; 3]) -> [u8; 3] {
    [
        (r as u16 * 3 / 5) as u8,
        (g as u16 * 3 / 5) as u8,
        (b as u16 * 3 / 5) as u8,
    ]
}

fn resolve(color: Color, palette: &Palette) -> [u8; 3] {
    match color {
        Color::Spec(rgb) => [rgb.r, rgb.g, rgb.b],
        Color::Indexed(i) => indexed_rgb(i, palette),
        Color::Named(named) => named_rgb(named, palette),
    }
}

/// Apply a cell-addressed pointer event to the terminal's own selection,
/// placed with the *live* scroll offset — a caller's snapshot may lag it.
/// Whether the event is the terminal's to apply at all — or the child's, when
/// it reads the mouse — is decided by the terminal thread before it gets here.
pub(crate) fn apply_pointer<T: EventListener>(term: &mut Term<T>, pointer: PointerEvent) {
    if let Some(op) = pointer_select(&pointer, term.grid().display_offset()) {
        apply_select(term, op);
    }
}

/// Apply a selection change to the terminal's own grid-anchored selection. The
/// grid `Point` is what the emulator rotates on every scroll, so the highlight
/// (derived by [`selected_spans`]) follows the text; a bare click clears it.
pub(crate) fn apply_select<T: EventListener>(term: &mut Term<T>, op: SelectOp) {
    use alacritty_terminal::index::{Column, Line, Point, Side};
    use alacritty_terminal::selection::{Selection, SelectionType};
    let to_side = |s| match s {
        SelectSide::Left => Side::Left,
        SelectSide::Right => Side::Right,
    };
    match op {
        SelectOp::Start { line, col, side } => {
            let at = Point::new(Line(line), Column(col));
            term.selection = Some(Selection::new(SelectionType::Simple, at, to_side(side)));
        }
        SelectOp::Update { line, col, side } => {
            if let Some(sel) = term.selection.as_mut() {
                sel.update(Point::new(Line(line), Column(col)), to_side(side));
            }
        }
        SelectOp::Range {
            line0,
            col0,
            line1,
            col1,
        } => {
            let anchor = Point::new(Line(line0), Column(col0));
            let mut sel = Selection::new(SelectionType::Simple, anchor, Side::Left);
            sel.update(Point::new(Line(line1), Column(col1)), Side::Right);
            term.selection = Some(sel);
        }
        SelectOp::Clear => term.selection = None,
    }
}

/// The terminal's own selection as inclusive per-row column spans in visible
/// coordinates. The selection is anchored in the grid and the emulator rotates
/// it on every scroll, so mapping each covered grid line to its viewport row
/// (`line - first_line`, matching the cell loop) makes the highlight follow the
/// text through both scrollback and application-driven scroll. Rows outside the
/// viewport are dropped, so a selection scrolled partly off clips cleanly.
fn selected_spans<T: EventListener>(
    term: &Term<T>,
    first_line: i32,
    cols: u16,
    rows: u16,
) -> Vec<(u16, u16, u16)> {
    let Some(range) = term.selection.as_ref().and_then(|s| s.to_range(term)) else {
        return Vec::new();
    };
    let last = cols.saturating_sub(1) as usize;
    let mut spans = Vec::new();
    // Only the rows on screen can be highlighted, so walk the selection clamped
    // to the viewport (`first_line..first_line + rows`) rather than its full
    // height — a select-all over deep scrollback would otherwise spin thousands
    // of off-screen iterations on every frame. An off-screen selection makes the
    // range empty, so nothing is pushed.
    let top = range.start.line.0.max(first_line);
    let bottom = range.end.line.0.min(first_line + i32::from(rows) - 1);
    for line in top..=bottom {
        let row = (line - first_line) as u16;
        // Block selection keeps the same columns on every row; a normal
        // selection runs the first row to the edge, fills the middle, and stops
        // the last row at its column.
        let (c0, c1) = if range.is_block || range.start.line.0 == range.end.line.0 {
            (range.start.column.0, range.end.column.0)
        } else if line == range.start.line.0 {
            (range.start.column.0, last)
        } else if line == range.end.line.0 {
            (0, range.end.column.0)
        } else {
            (0, last)
        };
        spans.push((row, c0.min(last) as u16, c1.min(last) as u16));
    }
    spans
}

/// Snapshot the visible grid into a [`Screen`] with resolved colours and the
/// cursor (FR4). Wide-char spacer cells are dropped; the wide glyph keeps its
/// own column.
pub(crate) fn snapshot<T: EventListener>(term: &Term<T>, palette: &Palette) -> Screen {
    let cols = term.columns() as u16;
    let rows = term.screen_lines() as u16;
    let mut lines = vec![vec![palette.blank_cell(); cols as usize]; rows as usize];

    let content = term.renderable_content();
    let first_line = -(content.display_offset as i32);
    let cursor_shape = content.cursor.shape;
    let cursor_point = content.cursor.point;
    let mut hyperlinks = HyperlinkRuns::default();

    for indexed in content.display_iter {
        let row = indexed.point.line.0 - first_line;
        let col = indexed.point.column.0;
        if row < 0 || row as u16 >= rows || col as u16 >= cols {
            continue;
        }
        let cell = indexed.cell;
        // A wide glyph's spacer carries the link too: read it before the
        // spacer is dropped, or the span would break at every wide glyph.
        hyperlinks.push(row as u16, col as u16, cell.hyperlink());
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let bold = cell.flags.intersects(Flags::BOLD | Flags::DIM_BOLD);
        let mut fg = resolve(cell.fg, palette);
        // The faint/dim attribute (SGR 2) darkens the foreground. Without this
        // dim default-fg text — like Claude's greyed suggestions — rendered at
        // full intensity, i.e. near-white.
        if cell.flags.contains(Flags::DIM) {
            fg = dim(fg);
        }
        let mut bg = resolve(cell.bg, palette);
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        let c = if cell.flags.contains(Flags::HIDDEN) {
            ' '
        } else {
            cell.c
        };
        lines[row as usize][col] = ScreenCell { c, fg, bg, bold };
    }

    let cursor = (cursor_shape != CursorShape::Hidden)
        .then(|| {
            let row = cursor_point.line.0 - first_line;
            (row >= 0 && (row as u16) < rows && (cursor_point.column.0 as u16) < cols)
                .then_some((cursor_point.column.0 as u16, row as u16))
        })
        .flatten();

    Screen {
        cols,
        rows,
        lines,
        cursor,
        scrolled: content.display_offset > 0,
        display_offset: content.display_offset,
        bracketed_paste: term.mode().contains(TermMode::BRACKETED_PASTE),
        mouse_reporting: mouse_reporting(*term.mode()),
        selection: selected_spans(term, first_line, cols, rows),
        hyperlinks: hyperlinks.finish(),
        default_bg: palette.background,
        cursor_color: palette.cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::vte::ansi::Processor;

    #[test]
    fn named_schemes_resolve_and_unknown_is_none() {
        // Two darks, two lights — fg/bg from each published specification.
        let sd = Palette::named("solarized-dark").expect("known scheme");
        assert_eq!(sd.background, [0x00, 0x2b, 0x36]);
        assert_eq!(sd.foreground, [0x83, 0x94, 0x96]);
        let sl = Palette::named("solarized-light").expect("known scheme");
        assert_eq!(sl.background, [0xfd, 0xf6, 0xe3]);
        assert_eq!(sl.ansi, sd.ansi, "solarized variants share their accents");
        let gd = Palette::named("gruvbox-dark").expect("known scheme");
        assert_eq!(gd.background, [0x28, 0x28, 0x28]);
        let gl = Palette::named("gruvbox-light").expect("known scheme");
        assert_eq!(gl.background, [0xfb, 0xf1, 0xc7]);
        assert_ne!(gl.ansi, gd.ansi, "gruvbox brights differ per variant");
        // Every scheme keeps the cursor on its foreground.
        for p in [&sd, &sl, &gd, &gl] {
            assert_eq!(p.cursor, p.foreground);
        }
        assert_eq!(Palette::named("no-such-scheme"), None);
    }

    /// The OSC 8 target rides the snapshot as a span over the cells it covers,
    /// so a link whose label is `#76` can still resolve to its full URL — the
    /// text alone has nothing to detect.
    #[test]
    fn snapshot_carries_an_osc8_hyperlink_as_a_span_over_its_cells() {
        use alacritty_terminal::event::VoidListener;
        let mut term = Term::new(Config::default(), &TermSize::new(20, 2), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(
            &mut term,
            b"see \x1b]8;;https://ex.io/issues/76\x1b\\#76\x1b]8;;\x1b\\ now",
        );
        let screen = snapshot(&term, &Palette::default());
        assert_eq!(
            screen.hyperlinks,
            vec![HyperlinkSpan {
                row: 0,
                start: 4,
                end: 7,
                uri: "https://ex.io/issues/76".into(),
            }]
        );
        // The label is what the grid shows; the URI lives only on the span.
        let text: String = screen.lines[0][4..7].iter().map(|c| c.c).collect();
        assert_eq!(text, "#76");
    }

    /// One hyperlink id printed in two places is two spans: a click on either
    /// run opens the same target, and neither underline bleeds across the gap.
    #[test]
    fn a_hyperlink_broken_by_plain_cells_is_two_spans() {
        use alacritty_terminal::event::VoidListener;
        let mut term = Term::new(Config::default(), &TermSize::new(20, 2), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(
            &mut term,
            b"\x1b]8;id=a;https://ex.io\x1b\\ab\x1b]8;;\x1b\\--\x1b]8;id=a;https://ex.io\x1b\\cd\x1b]8;;\x1b\\",
        );
        let spans = snapshot(&term, &Palette::default()).hyperlinks;
        let cells: Vec<(u16, u16, u16)> = spans.iter().map(|s| (s.row, s.start, s.end)).collect();
        assert_eq!(cells, vec![(0, 0, 2), (0, 4, 6)]);
        assert!(spans.iter().all(|s| s.uri == "https://ex.io"));
    }

    /// A link that wraps at the right edge is one span per row: the underline
    /// is drawn per row, and a span never crosses the line break.
    #[test]
    fn a_hyperlink_wrapping_the_row_edge_is_one_span_per_row() {
        use alacritty_terminal::event::VoidListener;
        let mut term = Term::new(Config::default(), &TermSize::new(10, 2), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(
            &mut term,
            b"\x1b]8;;https://ex.io\x1b\\abcdefghijkl\x1b]8;;\x1b\\",
        );
        let spans = snapshot(&term, &Palette::default()).hyperlinks;
        let cells: Vec<(u16, u16, u16)> = spans.iter().map(|s| (s.row, s.start, s.end)).collect();
        assert_eq!(cells, vec![(0, 0, 10), (1, 0, 2)]);
    }

    /// A wide glyph inside a link keeps the span whole: its spacer cell is not
    /// rendered, but it is not a gap in the link either.
    #[test]
    fn a_wide_glyph_does_not_split_a_hyperlink_span() {
        use alacritty_terminal::event::VoidListener;
        let mut term = Term::new(Config::default(), &TermSize::new(20, 2), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(
            &mut term,
            "\x1b]8;;https://ex.io\x1b\\a日b\x1b]8;;\x1b\\".as_bytes(),
        );
        let spans = snapshot(&term, &Palette::default()).hyperlinks;
        let cells: Vec<(u16, u16, u16)> = spans.iter().map(|s| (s.row, s.start, s.end)).collect();
        assert_eq!(cells, vec![(0, 0, 4)]);
    }

    /// Plain text — a printed URL included — carries no OSC 8 span; finding
    /// that URL stays the plaintext scan's job.
    #[test]
    fn plain_text_carries_no_hyperlink_span() {
        use alacritty_terminal::event::VoidListener;
        let mut term = Term::new(Config::default(), &TermSize::new(20, 2), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"see https://ex.io");
        assert!(snapshot(&term, &Palette::default()).hyperlinks.is_empty());
    }

    #[test]
    fn snapshot_tracks_bracketed_paste_mode() {
        use alacritty_terminal::event::VoidListener;
        let mut term = Term::new(Config::default(), &TermSize::new(20, 5), VoidListener);
        let mut parser: Processor = Processor::new();
        let palette = Palette::default();
        assert!(!snapshot(&term, &palette).bracketed_paste);
        // DECSET 2004 turns it on; the matching reset turns it off again.
        parser.advance(&mut term, b"\x1b[?2004h");
        assert!(snapshot(&term, &palette).bracketed_paste);
        parser.advance(&mut term, b"\x1b[?2004l");
        assert!(!snapshot(&term, &palette).bracketed_paste);
    }

    /// The bug behind reopening the "selection follows scroll" work: in the
    /// alternate screen a TUI (Claude, vim, less) scrolls its *own* content —
    /// `display_offset` never moves — so a selection anchored to a viewport row
    /// froze in place. Anchored to the terminal's own selection, which the
    /// emulator rotates on every grid scroll, the highlight must ride the text.
    #[test]
    fn a_selection_rides_an_app_driven_scroll_in_the_alt_screen() {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::index::{Column, Line, Point, Side};
        use alacritty_terminal::selection::{Selection, SelectionType};
        let mut term = Term::new(Config::default(), &TermSize::new(10, 4), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"\x1b[?1049h"); // enter the alternate screen
        parser.advance(&mut term, b"one\r\ntwo\r\nthree\r\nfour");
        // Select "three" on viewport row 2.
        let mut sel = Selection::new(
            SelectionType::Simple,
            Point::new(Line(2), Column(0)),
            Side::Left,
        );
        sel.update(Point::new(Line(2), Column(4)), Side::Right);
        term.selection = Some(sel);
        assert_eq!(
            snapshot(&term, &Palette::default()).selection,
            vec![(2, 0, 4)],
            "row 2 is highlighted before the scroll"
        );
        // The app scrolls its content up one line (a linefeed on the bottom row),
        // as a TUI does: the grid scrolls though display_offset stays 0.
        parser.advance(&mut term, b"\r\nfive");
        assert_eq!(
            snapshot(&term, &Palette::default()).selection,
            vec![(1, 0, 4)],
            "the highlight followed the text up to row 1"
        );
    }

    /// The shipped behaviour, now via the native selection: scrolling the
    /// viewport back into scrollback rides the highlight down with the text.
    #[test]
    fn a_selection_rides_scrollback_scroll() {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::grid::Scroll;
        use alacritty_terminal::index::{Column, Line, Point, Side};
        use alacritty_terminal::selection::{Selection, SelectionType};
        let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
        let mut parser: Processor = Processor::new();
        // Six rows into a three-row screen leaves three lines of scrollback.
        parser.advance(&mut term, b"l0\r\nl1\r\nl2\r\nl3\r\nl4\r\nl5");
        // Select the live row 0 ("l3").
        let mut sel = Selection::new(
            SelectionType::Simple,
            Point::new(Line(0), Column(0)),
            Side::Left,
        );
        sel.update(Point::new(Line(0), Column(1)), Side::Right);
        term.selection = Some(sel);
        assert_eq!(
            snapshot(&term, &Palette::default()).selection,
            vec![(0, 0, 1)],
            "row 0 is highlighted at the live tail"
        );
        // Scroll one line up into history; the same text now sits one row lower.
        term.scroll_display(Scroll::Delta(1));
        assert_eq!(
            snapshot(&term, &Palette::default()).selection,
            vec![(1, 0, 1)],
            "the highlight followed the text down to row 1"
        );
    }

    /// A multi-row selection maps to inclusive per-row column spans: the first
    /// row runs from its column to the edge, middle rows are full width, the last
    /// row stops at its column — what the highlight draw pass paints.
    #[test]
    fn snapshot_reports_a_multi_row_selection_as_per_row_spans() {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::index::{Column, Line, Point, Side};
        use alacritty_terminal::selection::{Selection, SelectionType};
        let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"aaaaaaaaaa\r\nbbbbbbbbbb\r\ncccccccccc");
        let mut sel = Selection::new(
            SelectionType::Simple,
            Point::new(Line(0), Column(2)),
            Side::Left,
        );
        sel.update(Point::new(Line(2), Column(1)), Side::Right);
        term.selection = Some(sel);
        assert_eq!(
            snapshot(&term, &Palette::default()).selection,
            vec![(0, 2, 9), (1, 0, 9), (2, 0, 1)],
            "first row to the edge, middle full width, last row to its column"
        );
    }

    /// A selection straddling the viewport's top edge — its start scrolled up
    /// into history — highlights only the on-screen rows. This is what makes a
    /// copy of it take only the visible text (the clip that used to live in the
    /// app's `visible_spans`).
    #[test]
    fn a_selection_scrolled_partly_off_clips_to_the_visible_rows() {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::index::{Column, Line, Point, Side};
        use alacritty_terminal::selection::{Selection, SelectionType};
        let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
        let mut parser: Processor = Processor::new();
        // Six rows into a three-row screen leaves three lines of scrollback; the
        // visible rows hold l3 / l4 / l5, and l1 / l2 sit above at Line(-2)/(-1).
        parser.advance(&mut term, b"l0\r\nl1\r\nl2\r\nl3\r\nl4\r\nl5");
        // Select from a history line (Line -2 = "l1") down to a visible one
        // (Line 1 = "l4").
        let mut sel = Selection::new(
            SelectionType::Simple,
            Point::new(Line(-2), Column(0)),
            Side::Left,
        );
        sel.update(Point::new(Line(1), Column(1)), Side::Right);
        term.selection = Some(sel);
        // Only the two on-screen rows appear; the off-top history rows clip out.
        assert_eq!(
            snapshot(&term, &Palette::default()).selection,
            vec![(0, 0, 9), (1, 0, 1)],
            "the highlight covers only the visible rows of the selection"
        );
    }

    #[test]
    fn dim_scales_colours_to_about_three_fifths() {
        assert_eq!(dim(DEFAULT_FG), [124, 124, 124]);
        assert_eq!(dim([0, 0, 0]), [0, 0, 0]);
        assert_eq!(dim([255, 255, 255]), [153, 153, 153]);
    }

    #[test]
    fn snapshot_darkens_dim_foreground() {
        use alacritty_terminal::event::VoidListener;
        let mut term = Term::new(Config::default(), &TermSize::new(20, 2), VoidListener);
        let mut parser: Processor = Processor::new();
        // SGR 2 = faint/dim, then a glyph on the default foreground.
        parser.advance(&mut term, b"\x1b[2mX");
        let cell = snapshot(&term, &Palette::default()).lines[0][0];
        assert_eq!(cell.c, 'X');
        assert_eq!(cell.fg, dim(DEFAULT_FG));
        assert!(
            cell.fg[0] < DEFAULT_FG[0],
            "dim must darken the default foreground"
        );
    }

    #[test]
    fn colour_resolution_covers_the_256_palette() {
        let palette = Palette::default();
        // ANSI 16.
        assert_eq!(indexed_rgb(0, &palette), [0x00, 0x00, 0x00]);
        assert_eq!(indexed_rgb(15, &palette), [0xff, 0xff, 0xff]);
        // First cube entry (16) is black; last (231) is white.
        assert_eq!(indexed_rgb(16, &palette), [0, 0, 0]);
        assert_eq!(indexed_rgb(231, &palette), [255, 255, 255]);
        // Grayscale ramp endpoints.
        assert_eq!(indexed_rgb(232, &palette), [8, 8, 8]);
        assert_eq!(indexed_rgb(255, &palette), [238, 238, 238]);
        // Spec passes through; named foreground/background hit the defaults.
        assert_eq!(
            resolve(
                Color::Spec(alacritty_terminal::vte::ansi::Rgb { r: 1, g: 2, b: 3 }),
                &palette
            ),
            [1, 2, 3]
        );
        assert_eq!(
            resolve(Color::Named(NamedColor::Background), &palette),
            DEFAULT_BG
        );
    }

    #[test]
    fn a_custom_palette_recolours_named_indexed_and_default_cells() {
        let mut ansi = ANSI16;
        ansi[1] = [0xaa, 0x00, 0x00];
        let palette = Palette {
            foreground: [0x10, 0x20, 0x30],
            background: [0xfa, 0xfb, 0xfc],
            cursor: [0x01, 0x02, 0x03],
            ansi,
        };

        // The configurable 16 drive both the named and the indexed forms.
        assert_eq!(
            resolve(Color::Named(NamedColor::Red), &palette),
            [0xaa, 0x00, 0x00]
        );
        assert_eq!(resolve(Color::Indexed(1), &palette), [0xaa, 0x00, 0x00]);
        assert_eq!(
            resolve(Color::Named(NamedColor::Foreground), &palette),
            [0x10, 0x20, 0x30]
        );
        assert_eq!(
            resolve(Color::Named(NamedColor::Cursor), &palette),
            [0x01, 0x02, 0x03]
        );
        // The 256-cube stays computed, untouched by the overrides.
        assert_eq!(resolve(Color::Indexed(231), &palette), [255, 255, 255]);
        // The dim variants keep their fixed, hand-tuned values.
        assert_eq!(
            resolve(Color::Named(NamedColor::DimRed), &palette),
            [0x88, 0x22, 0x22]
        );

        // A snapshot carries the palette's chrome colours and blanks.
        use alacritty_terminal::event::VoidListener;
        let term = Term::new(Config::default(), &TermSize::new(4, 2), VoidListener);
        let screen = snapshot(&term, &palette);
        assert_eq!(screen.default_bg, [0xfa, 0xfb, 0xfc]);
        assert_eq!(screen.cursor_color, [0x01, 0x02, 0x03]);
        assert_eq!(screen.lines[0][0].fg, [0x10, 0x20, 0x30]);
        assert_eq!(screen.lines[0][0].bg, [0xfa, 0xfb, 0xfc]);
    }

    // --- cell-addressed pointer over a real grid --------------------------

    /// A press then a drag over the text leaves the terminal's own selection
    /// holding exactly the dragged text — the observable this rung has before
    /// the child can be handed the mouse.
    #[test]
    fn a_pointer_press_and_drag_select_the_text_between_them() {
        use alacritty_terminal::event::VoidListener;
        use termherd_core::PointerKind;
        let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"hello wide\r\nworld");
        apply_pointer(&mut term, PointerEvent::left(PointerKind::Press, 6, 0));
        apply_pointer(&mut term, PointerEvent::left(PointerKind::Drag, 4, 1));
        assert_eq!(
            snapshot(&term, &Palette::default()).selection,
            vec![(0, 6, 9), (1, 0, 4)],
            "row 0 from the press to the edge, row 1 up to the drag cell"
        );
        assert_eq!(
            term.selection_to_string().as_deref(),
            Some("wide\nworld"),
            "the dragged text is what a copy reads"
        );
    }

    /// A bare click drops the highlight, as it does under a real pointer, so an
    /// agent can dismiss a selection it made without a second tool.
    #[test]
    fn a_pointer_click_clears_the_selection() {
        use alacritty_terminal::event::VoidListener;
        use termherd_core::PointerKind;
        let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"hello");
        apply_pointer(&mut term, PointerEvent::left(PointerKind::Press, 0, 0));
        apply_pointer(&mut term, PointerEvent::left(PointerKind::Drag, 4, 0));
        assert!(!snapshot(&term, &Palette::default()).selection.is_empty());
        apply_pointer(&mut term, PointerEvent::left(PointerKind::Click, 2, 0));
        assert!(
            snapshot(&term, &Palette::default()).selection.is_empty(),
            "the click cleared it"
        );
    }

    /// The pointer is addressed in visible rows; with the viewport scrolled up
    /// into history the terminal anchors the selection to the text under that
    /// row, not to the live line the row number would name at the tail.
    #[test]
    fn a_pointer_over_a_scrolled_viewport_selects_the_text_under_the_row() {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::grid::Scroll;
        use termherd_core::PointerKind;
        let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
        let mut parser: Processor = Processor::new();
        // Six rows into a three-row screen: three lines of scrollback.
        parser.advance(&mut term, b"l0\r\nl1\r\nl2\r\nl3\r\nl4\r\nl5");
        // Scrolled up two lines, the visible rows show l1 / l2 / l3.
        term.scroll_display(Scroll::Delta(2));
        apply_pointer(&mut term, PointerEvent::left(PointerKind::Press, 0, 1));
        apply_pointer(&mut term, PointerEvent::left(PointerKind::Drag, 1, 1));
        assert_eq!(
            term.selection_to_string().as_deref(),
            Some("l2"),
            "visible row 1 is l2 while scrolled, not l4"
        );
    }

    // --- the child reads the mouse ------------------------------------------

    #[test]
    fn snapshot_tracks_the_mouse_reporting_ladder() {
        use alacritty_terminal::event::VoidListener;
        let mut term = Term::new(Config::default(), &TermSize::new(20, 5), VoidListener);
        let mut parser: Processor = Processor::new();
        let palette = Palette::default();
        assert_eq!(snapshot(&term, &palette).mouse_reporting, None);
        for (set, reset, expected) in [
            (b"\x1b[?1000h", b"\x1b[?1000l", MouseReporting::Click),
            (b"\x1b[?1002h", b"\x1b[?1002l", MouseReporting::Drag),
            (b"\x1b[?1003h", b"\x1b[?1003l", MouseReporting::Motion),
        ] {
            parser.advance(&mut term, set);
            assert_eq!(snapshot(&term, &palette).mouse_reporting, Some(expected));
            parser.advance(&mut term, reset);
            assert_eq!(
                snapshot(&term, &palette).mouse_reporting,
                None,
                "{expected:?} reset"
            );
        }
    }
}
