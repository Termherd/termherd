//! Pure pointer geometry for the terminal grid: mapping a pointer to a cell and
//! its selection side, resolving the word or link under a cell, and reading the
//! text of a highlighted span. The selection itself is anchored and rotated by
//! the terminal (`termherd_pty`), which carries the highlighted spans on each
//! [`Screen`]; the functions here only translate pointer positions and read the
//! resulting spans, so every one is exhaustively unit-testable.

use iced::{Rectangle, mouse};
use termherd_core::SelectSide;
use termherd_pty::Screen;

/// A link the pointer is hovering with Ctrl/Cmd held — what to highlight and,
/// on click, what to open.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct HoverLink {
    pub(super) row: u16,
    pub(super) start: u16,
    pub(super) end: u16,
    pub(super) url: String,
}

/// The grid cell under the cursor, if any.
pub(super) fn cell_at(
    cursor: mouse::Cursor,
    bounds: Rectangle,
    screen: &Screen,
) -> Option<(u16, u16)> {
    let p = cursor.position_in(bounds)?;
    let cols = screen.cols.max(1);
    let rows = screen.rows.max(1);
    let cw = bounds.width / cols as f32;
    let ch = bounds.height / rows as f32;
    if cw <= 0.0 || ch <= 0.0 {
        return None;
    }
    let c = (p.x / cw).floor().clamp(0.0, (cols - 1) as f32) as u16;
    let r = (p.y / ch).floor().clamp(0.0, (rows - 1) as f32) as u16;
    Some((c, r))
}

/// Which half of its cell the pointer sits in. A press past a cell's centre
/// starts/extends the selection *through* that cell (right side); before it, the
/// selection stops at the cell's left edge — the terminal's own left/right
/// notion, so a drag feels precise rather than snapping to whole cells.
pub(super) fn cell_side(cursor: mouse::Cursor, bounds: Rectangle, cols: u16) -> SelectSide {
    let frac = cursor.position_in(bounds).map_or(0.0, |p| {
        let cw = bounds.width / cols.max(1) as f32;
        if cw > 0.0 { (p.x / cw).fract() } else { 0.0 }
    });
    if frac >= 0.5 {
        SelectSide::Right
    } else {
        SelectSide::Left
    }
}

/// The link under grid cell `(col, row)`, if any. Builds the row's text
/// from its cells — one char per cell, so a `core::links` char-index span maps
/// straight onto columns — and returns the span containing `col`.
pub(super) fn link_at(screen: &Screen, col: u16, row: u16) -> Option<HoverLink> {
    let line = screen.lines.get(row as usize)?;
    let text: String = line.iter().map(|cell| cell.c).collect();
    let span = termherd_core::links::detect(&text)
        .into_iter()
        .find(|span| span.contains(&(col as usize)))?;
    let url: String = line[span.clone()].iter().map(|cell| cell.c).collect();
    Some(HoverLink {
        row,
        start: span.start as u16,
        end: span.end as u16,
        url,
    })
}

/// Whether a character belongs to a double-click "word". Alphanumerics
/// plus the punctuation that holds filenames and paths together, so a unit like
/// `~/src/main.rs:42` selects whole; whitespace and bracketing punctuation
/// (quotes, parens, commas) are boundaries.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '\\' | '~' | ':' | '@' | '+')
}

/// The word / filename under grid cell `(col, row)` as an inclusive cell range
/// `(anchor, head)`, or `None` when the cell is not part of a word (e.g. blank).
/// A word is the maximal run of [`is_word_char`] cells around `col` — this is
/// what a double-click selects.
pub(super) fn word_at(screen: &Screen, col: u16, row: u16) -> Option<((u16, u16), (u16, u16))> {
    let line = screen.lines.get(row as usize)?;
    let here = col as usize;
    if !line.get(here).is_some_and(|cell| is_word_char(cell.c)) {
        return None;
    }
    let mut start = here;
    while start > 0 && is_word_char(line[start - 1].c) {
        start -= 1;
    }
    let mut end = here;
    while end + 1 < line.len() && is_word_char(line[end + 1].c) {
        end += 1;
    }
    Some(((start as u16, row), (end as u16, row)))
}

/// The text of the highlighted selection carried on `screen` — one line per
/// covered row, trailing blanks trimmed, joined by newlines. This is what a drag
/// copies; the terminal already clipped the spans to the visible rows, so a
/// selection scrolled partly out of view copies only what is on screen.
pub(super) fn selection_text(screen: &Screen) -> String {
    spans_text(screen, &screen.selection)
}

/// The text of a single-row word / filename range — what a double-click copies
/// before its native selection has been echoed back on a snapshot.
pub(super) fn word_text(screen: &Screen, anchor: (u16, u16), head: (u16, u16)) -> String {
    spans_text(screen, &[(anchor.1, anchor.0, head.0)])
}

/// Read inclusive `(row, c0, c1)` spans off the grid, trimming each row's
/// trailing blanks and joining rows with newlines.
fn spans_text(screen: &Screen, spans: &[(u16, u16, u16)]) -> String {
    let mut out = String::new();
    let last = spans.len().saturating_sub(1);
    for (i, (r, c0, c1)) in spans.iter().enumerate() {
        if let Some(line) = screen.lines.get(*r as usize) {
            let c0 = *c0 as usize;
            let c1 = (*c1 as usize).min(line.len().saturating_sub(1));
            if c0 <= c1 {
                let row: String = line[c0..=c1].iter().map(|cell| cell.c).collect();
                out.push_str(row.trim_end());
            }
        }
        if i != last {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use termherd_pty::ScreenCell;

    /// A single-row screen holding `line`, one char per cell.
    fn screen_from(line: &str) -> Screen {
        let cells: Vec<ScreenCell> = line
            .chars()
            .map(|c| ScreenCell {
                c,
                fg: [0, 0, 0],
                bg: [0, 0, 0],
                bold: false,
            })
            .collect();
        Screen {
            cols: cells.len() as u16,
            rows: 1,
            lines: vec![cells],
            cursor: None,
            scrolled: false,
            display_offset: 0,
            bracketed_paste: false,
            selection: Vec::new(),
        }
    }

    /// A screen from one string per row, blank-padded to the widest row, with a
    /// given set of highlighted spans.
    fn grid_with_selection(rows: &[&str], selection: Vec<(u16, u16, u16)>) -> Screen {
        let cell = |c| ScreenCell {
            c,
            fg: [0, 0, 0],
            bg: [0, 0, 0],
            bold: false,
        };
        let cols = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as u16;
        let lines = rows
            .iter()
            .map(|r| {
                let mut cells: Vec<ScreenCell> = r.chars().map(cell).collect();
                cells.resize(cols as usize, cell(' '));
                cells
            })
            .collect();
        Screen {
            cols,
            rows: rows.len() as u16,
            lines,
            cursor: None,
            scrolled: false,
            display_offset: 0,
            bracketed_paste: false,
            selection,
        }
    }

    #[test]
    fn selection_text_reads_and_trims_the_covered_rows() {
        // A multi-row selection carried on the screen: first row to its column
        // span, middle full width (trailing blanks trimmed), last row to its
        // column — the newlines join exactly the covered rows.
        let screen = grid_with_selection(
            &["AAAA", "BB  ", "CCCC"],
            vec![(0, 1, 3), (1, 0, 3), (2, 0, 1)],
        );
        assert_eq!(selection_text(&screen), "AAA\nBB\nCC");
    }

    #[test]
    fn selection_text_is_empty_without_a_selection() {
        let screen = grid_with_selection(&["AAAA"], Vec::new());
        assert_eq!(selection_text(&screen), "");
    }

    #[test]
    fn word_text_reads_a_single_row_range() {
        let screen = screen_from("see src/main.rs now");
        // cols 4..=14 is `src/main.rs`.
        assert_eq!(word_text(&screen, (4, 0), (14, 0)), "src/main.rs");
    }

    #[test]
    fn cell_side_splits_the_cell_at_its_centre() {
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 10.0,
        };
        // 4 columns → 10px each. x=2 is the left half of cell 0, x=8 the right.
        let left = mouse::Cursor::Available(iced::Point::new(2.0, 5.0));
        let right = mouse::Cursor::Available(iced::Point::new(8.0, 5.0));
        assert!(matches!(cell_side(left, bounds, 4), SelectSide::Left));
        assert!(matches!(cell_side(right, bounds, 4), SelectSide::Right));
    }

    #[test]
    fn link_at_finds_the_url_under_a_column() {
        // the column maps onto the detected span and yields its URL.
        let screen = screen_from("see https://ex.io now");
        let link = link_at(&screen, 6, 0).expect("column 6 is inside the URL");
        assert_eq!(link.url, "https://ex.io");
        assert_eq!((link.start, link.end), (4, 17));
        // A column off the URL has no link.
        assert!(link_at(&screen, 0, 0).is_none());
    }

    #[test]
    fn word_at_spans_a_filename_run() {
        // a path/filename is one word — letters, digits and the joining
        // punctuation (`/ . :`) all count, blanks bound it.
        let screen = screen_from("see src/main.rs:42 now");
        // Column 8 ('m') sits inside the `src/main.rs:42` run (cols 4..=17).
        assert_eq!(word_at(&screen, 8, 0), Some(((4, 0), (17, 0))));
        // A blank cell is not part of any word.
        assert_eq!(word_at(&screen, 3, 0), None);
        // A column past the line has no word.
        assert_eq!(word_at(&screen, 99, 0), None);
    }
}
