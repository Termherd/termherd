//! Pure pointer/selection geometry for the terminal grid: mapping a pointer to
//! a cell, resolving the word or link under a cell, ordering two cells in
//! reading order, and turning a selection into its per-row column spans and
//! copied text. No rendering, no iced state — just arithmetic over a [`Screen`]
//! snapshot, so every function here is exhaustively unit-testable.

use iced::{Rectangle, mouse};
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

/// Order two cells in reading order (row, then column).
pub(super) fn ordered(a: (u16, u16), b: (u16, u16)) -> ((u16, u16), (u16, u16)) {
    if (a.1, a.0) <= (b.1, b.0) {
        (a, b)
    } else {
        (b, a)
    }
}

/// The selected column span `[c0, c1]` on row `r` of an ordered selection.
pub(super) fn selection_span(start: (u16, u16), end: (u16, u16), r: u16, cols: u16) -> (u16, u16) {
    let last = cols.saturating_sub(1);
    if start.1 == end.1 {
        (start.0.min(end.0), start.0.max(end.0))
    } else if r == start.1 {
        (start.0, last)
    } else if r == end.1 {
        (0, end.0)
    } else {
        (0, last)
    }
}

/// Extract the selected text from the visible grid, trimming trailing blanks.
pub(super) fn selection_text(screen: &Screen, a: (u16, u16), b: (u16, u16)) -> String {
    let (start, end) = ordered(a, b);
    let mut out = String::new();
    for r in start.1..=end.1 {
        let Some(line) = screen.lines.get(r as usize) else {
            continue;
        };
        let (c0, c1) = selection_span(start, end, r, screen.cols);
        let c0 = c0 as usize;
        let c1 = (c1 as usize).min(line.len().saturating_sub(1));
        if c0 <= c1 {
            let row: String = line[c0..=c1].iter().map(|cell| cell.c).collect();
            out.push_str(row.trim_end());
        }
        if r != end.1 {
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
            bracketed_paste: false,
        }
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
