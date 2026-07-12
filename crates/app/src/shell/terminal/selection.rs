//! Pure pointer/selection geometry for the terminal grid: mapping a pointer to
//! a cell, resolving the word or link under a cell, anchoring a selection to
//! absolute scrollback lines so it follows the text through scroll, and turning
//! that selection into its per-row visible column spans and copied text. No
//! rendering, no iced state — just arithmetic over a [`Screen`] snapshot, so
//! every function here is exhaustively unit-testable.

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

/// The absolute scrollback line for a viewport `row` under the current scroll
/// `display_offset` — the coordinate a selection is anchored in so it tracks the
/// text through scroll. `render row = abs_line + display_offset` inverts it
/// (see [`visible_spans`]); the line is signed because a selection can sit above
/// the live tail (`row < display_offset`).
pub(super) fn abs_line(row: u16, display_offset: usize) -> i32 {
    row as i32 - display_offset as i32
}

/// The visible highlighted spans for a selection under the current scroll
/// `offset`: for each on-screen row the selection covers, `(row, c0, c1)` with
/// inclusive columns. `anchor`/`head` are absolute scrollback lines
/// (see [`abs_line`]); each is mapped back to a viewport row by adding `offset`,
/// and rows falling outside `0..rows` are omitted — so the highlight rides down
/// with the text as the viewport scrolls up and clips cleanly at the edges.
pub(super) fn visible_spans(
    anchor: (u16, i32),
    head: (u16, i32),
    offset: usize,
    cols: u16,
    rows: u16,
) -> Vec<(u16, u16, u16)> {
    let a = (anchor.0, anchor.1 + offset as i32);
    let b = (head.0, head.1 + offset as i32);
    // Reading order: by row, then column.
    let (start, end) = if (a.1, a.0) <= (b.1, b.0) {
        (a, b)
    } else {
        (b, a)
    };
    let last = cols.saturating_sub(1);
    (0..rows)
        .filter_map(|r| {
            let ri = i32::from(r);
            if ri < start.1 || ri > end.1 {
                return None;
            }
            let (c0, c1) = if start.1 == end.1 {
                (start.0.min(end.0), start.0.max(end.0))
            } else if ri == start.1 {
                (start.0, last)
            } else if ri == end.1 {
                (0, end.0)
            } else {
                (0, last)
            };
            Some((r, c0, c1))
        })
        .collect()
}

/// Extract the selected text from the visible grid, trimming trailing blanks.
/// `anchor`/`head` are absolute scrollback lines; only the on-screen portion of
/// the selection is read (the snapshot holds just the visible rows), so a
/// selection scrolled partly out of view copies only what is visible.
pub(super) fn selection_text(screen: &Screen, anchor: (u16, i32), head: (u16, i32)) -> String {
    let spans = visible_spans(
        anchor,
        head,
        screen.display_offset,
        screen.cols,
        screen.rows,
    );
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
            default_bg: [0x11, 0x13, 0x18],
            cursor_color: [0xd0, 0xd0, 0xd0],
        }
    }

    /// A screen from one string per row, blank-padded to the widest row.
    fn grid_from(rows: &[&str]) -> Screen {
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
            default_bg: [0x11, 0x13, 0x18],
            cursor_color: [0xd0, 0xd0, 0xd0],
        }
    }

    #[test]
    fn selection_text_reads_and_trims_the_covered_rows() {
        // A multi-row selection: the first row runs from its column to the edge,
        // middle rows are full width (trailing blanks trimmed), the last row
        // stops at its column — the newlines join exactly the covered rows.
        let screen = grid_from(&["AAAA", "BB  ", "CCCC"]);
        assert_eq!(selection_text(&screen, (1, 0), (1, 2)), "AAA\nBB\nCC");
    }

    #[test]
    fn selection_text_copies_only_the_visible_portion_when_scrolled_off() {
        // A selection whose absolute line sits above the viewport copies nothing
        // (only on-screen rows are in the snapshot).
        let screen = grid_from(&["AAAA", "BBBB"]);
        assert_eq!(selection_text(&screen, (0, -3), (3, -3)), "");
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

    #[test]
    fn visible_spans_covers_a_single_row_selection() {
        // one row, inclusive column span.
        assert_eq!(visible_spans((1, 0), (3, 0), 0, 4, 3), vec![(0, 1, 3)]);
    }

    #[test]
    fn visible_spans_fills_middle_rows_full_width() {
        // a multi-row selection: the first row runs to the edge, middle rows are
        // full width, the last row stops at its column — regardless of which
        // endpoint was the anchor (reading-order normalised).
        assert_eq!(
            visible_spans((2, 0), (1, 2), 0, 4, 3),
            vec![(0, 2, 3), (1, 0, 3), (2, 0, 1)]
        );
        assert_eq!(
            visible_spans((1, 2), (2, 0), 0, 4, 3),
            vec![(0, 2, 3), (1, 0, 3), (2, 0, 1)]
        );
    }

    #[test]
    fn visible_spans_ride_down_with_the_scroll_offset() {
        // the same absolute selection appears one row lower per line scrolled up.
        assert_eq!(visible_spans((0, 1), (3, 1), 0, 4, 3), vec![(1, 0, 3)]);
        assert_eq!(visible_spans((0, 1), (3, 1), 1, 4, 3), vec![(2, 0, 3)]);
    }

    #[test]
    fn visible_spans_clip_at_the_viewport_edges() {
        // rows mapping outside 0..rows are dropped.
        assert!(visible_spans((0, -2), (3, -2), 0, 4, 3).is_empty()); // above top
        assert!(visible_spans((0, 5), (3, 5), 0, 4, 3).is_empty()); // below bottom
        // straddling the top edge keeps only the visible rows.
        assert_eq!(
            visible_spans((0, -1), (1, 1), 0, 4, 3),
            vec![(0, 0, 3), (1, 0, 1)]
        );
    }

    proptest::proptest! {
        /// `abs_line` and the render mapping are inverse: capturing a viewport
        /// row at an offset and mapping it straight back returns that row.
        #[test]
        fn abs_line_round_trips(row in 0u16..500, offset in 0usize..500) {
            proptest::prop_assert_eq!(abs_line(row, offset) + offset as i32, i32::from(row));
        }

        /// Scrolling up `d` lines shifts a fully on-screen single-row highlight
        /// down by exactly `d` rows — never dropping or duplicating it.
        #[test]
        fn a_selection_shifts_by_the_offset_delta(row in 0u16..8, d in 0usize..8) {
            let rows = 16u16;
            let anchor = (0u16, abs_line(row, 0));
            let head = (3u16, abs_line(row, 0));
            proptest::prop_assert_eq!(visible_spans(anchor, head, 0, 4, rows), vec![(row, 0, 3)]);
            if (row as usize) + d < rows as usize {
                proptest::prop_assert_eq!(
                    visible_spans(anchor, head, d, 4, rows),
                    vec![(row + d as u16, 0, 3)]
                );
            }
        }
    }
}
