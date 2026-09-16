//! Pure pointer geometry for the terminal grid: mapping a pointer to a cell and
//! its selection side, resolving the word or link under a cell, and reading the
//! text of a highlighted span. The selection itself is anchored and rotated by
//! the terminal (`termherd_pty`), which carries the highlighted spans on each
//! [`Screen`]; the functions here only translate pointer positions and read the
//! resulting spans, so every one is exhaustively unit-testable.

use iced::{Point, Rectangle, mouse};
use termherd_core::{ProbeKind, SelectSide, TargetProbe};
use termherd_pty::Screen;

/// The grid cell under the cursor, if any — `None` when the pointer is off the
/// grid.
pub(super) fn cell_at(
    cursor: mouse::Cursor,
    bounds: Rectangle,
    screen: &Screen,
) -> Option<(u16, u16)> {
    let p = cursor.position_in(bounds)?;
    cell_of(p, bounds, screen)
}

/// The grid cell nearest the cursor, clamped to the edge when the pointer has
/// left the grid — what a gesture in progress needs, since a drag that crosses
/// the pane's edge must still end with its release on the border cell, as
/// xterm reports it, or the child is left holding a button forever.
pub(super) fn cell_nearest(
    cursor: mouse::Cursor,
    bounds: Rectangle,
    screen: &Screen,
) -> Option<(u16, u16)> {
    let p = cursor.position()?;
    cell_of(Point::new(p.x - bounds.x, p.y - bounds.y), bounds, screen)
}

/// The cell a point relative to the grid's origin falls in, clamped to the
/// grid on both axes.
fn cell_of(p: Point, bounds: Rectangle, screen: &Screen) -> Option<(u16, u16)> {
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

/// The clickable target under grid cell `(col, row)`, if any. An OSC 8
/// hyperlink over the cell answers first — its target is explicit, and the
/// text under it is only a label (`#76`), so there may be nothing to detect.
/// Otherwise the row's text is built from its cells — one char per cell, so a
/// `core::links` char-index span maps straight onto columns — and the span
/// containing `col` is returned.
///
/// A URL wins over a path: `https://ex.io/a/b` is path-shaped after its scheme,
/// and opening it in an editor is never what was meant. What comes back is a
/// *probe*, not an answer — only `core`, through the resolver port, can say
/// whether a path-shaped run is a file.
pub(super) fn target_at(screen: &Screen, col: u16, row: u16) -> Option<TargetProbe> {
    if let Some(link) = screen
        .hyperlinks
        .iter()
        .find(|link| link.row == row && (link.start..link.end).contains(&col))
    {
        return Some(TargetProbe {
            row,
            start: link.start,
            end: link.end,
            kind: ProbeKind::Url(link.uri.clone()),
        });
    }
    let line = screen.lines.get(row as usize)?;
    let text: String = line.iter().map(|cell| cell.c).collect();
    let here = col as usize;
    let read = |span: core::ops::Range<usize>| -> String {
        line.get(span)
            .map(|c| c.iter().map(|c| c.c).collect())
            .unwrap_or_default()
    };
    if let Some(span) = termherd_core::links::detect(&text)
        .into_iter()
        .find(|span| span.contains(&here))
    {
        return Some(TargetProbe {
            row,
            start: span.start as u16,
            end: span.end as u16,
            kind: ProbeKind::Url(read(span)),
        });
    }
    let span = termherd_core::paths::detect(&text)
        .into_iter()
        .find(|span| span.range.contains(&here))?;
    Some(TargetProbe {
        row,
        start: span.range.start as u16,
        end: span.range.end as u16,
        kind: ProbeKind::Path {
            candidate: read(span.target),
            line: span.line,
            col: span.col,
        },
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
    use termherd_pty::{HyperlinkSpan, ScreenCell};

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
            lines: vec![cells],
            ..Screen::blank(line.chars().count() as u16, 1)
        }
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

    /// The grid sits at an offset inside the window (a split, the sidebar),
    /// so a pointer off the grid is clamped relative to the grid's own origin
    /// on both axes, not the window's.
    #[test]
    fn cell_nearest_clamps_to_the_grid_from_its_own_origin() {
        let screen = Screen::blank(4, 2);
        let bounds = Rectangle {
            x: 100.0,
            y: 50.0,
            width: 40.0,
            height: 20.0,
        };
        let at = |x, y| mouse::Cursor::Available(Point::new(x, y));
        assert_eq!(
            cell_at(at(20.0, 10.0), bounds, &screen),
            None,
            "left of the grid"
        );
        assert_eq!(
            cell_nearest(at(20.0, 10.0), bounds, &screen),
            Some((0, 0)),
            "clamps to the top-left cell, not to the window's"
        );
        assert_eq!(
            cell_nearest(at(500.0, 500.0), bounds, &screen),
            Some((3, 1)),
            "and to the bottom-right cell past the far edges"
        );
        assert_eq!(
            cell_nearest(at(115.0, 65.0), bounds, &screen),
            Some((1, 1)),
            "inside, it is the cell under the pointer"
        );
        assert_eq!(
            cell_nearest(mouse::Cursor::Unavailable, bounds, &screen),
            None
        );
    }

    /// A grid with no extent on one axis has no cells, whichever axis it is.
    #[test]
    fn a_degenerate_grid_on_either_axis_has_no_cell() {
        let screen = Screen::blank(4, 2);
        let flat = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 0.0,
        };
        let thin = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 20.0,
        };
        let inside = mouse::Cursor::Available(Point::new(0.0, 0.0));
        assert_eq!(cell_nearest(inside, flat, &screen), None);
        assert_eq!(cell_nearest(inside, thin, &screen), None);
    }

    /// `screen` with one OSC 8 span laid over `start..end` of its only row.
    fn with_hyperlink(mut screen: Screen, start: u16, end: u16, uri: &str) -> Screen {
        screen.hyperlinks.push(HyperlinkSpan {
            row: 0,
            start,
            end,
            uri: uri.into(),
        });
        screen
    }

    #[test]
    fn target_at_resolves_a_hidden_hyperlink_from_its_span() {
        // `#76` shows nothing URL-shaped; the OSC 8 span carries the target.
        let screen = with_hyperlink(screen_from("see #76 now"), 4, 7, "https://ex.io/issues/76");
        let probe = target_at(&screen, 5, 0).expect("column 5 is inside the label");
        assert_eq!(probe.kind, ProbeKind::Url("https://ex.io/issues/76".into()));
        assert_eq!((probe.row, probe.start, probe.end), (0, 4, 7));
        // The cell just past the span is not part of the link.
        assert!(target_at(&screen, 7, 0).is_none());
    }

    #[test]
    fn a_hyperlink_span_outranks_the_text_it_covers() {
        // A printed URL wrapped in OSC 8 pointing elsewhere opens *elsewhere*:
        // the explicit target is what the program meant, the text is its label.
        let screen = with_hyperlink(
            screen_from("see https://ex.io now"),
            4,
            17,
            "https://other.io",
        );
        let probe = target_at(&screen, 6, 0).expect("column 6 is inside the URL");
        assert_eq!(probe.kind, ProbeKind::Url("https://other.io".into()));
    }

    #[test]
    fn a_hyperlink_span_on_another_row_does_not_claim_this_one() {
        let mut screen = with_hyperlink(screen_from("see #76 now"), 4, 7, "https://ex.io");
        screen.hyperlinks[0].row = 1;
        assert!(target_at(&screen, 5, 0).is_none());
    }

    #[test]
    fn target_at_finds_the_url_under_a_column() {
        // the column maps onto the detected span and yields its URL.
        let screen = screen_from("see https://ex.io now");
        let probe = target_at(&screen, 6, 0).expect("column 6 is inside the URL");
        assert_eq!(probe.kind, ProbeKind::Url("https://ex.io".into()));
        assert_eq!((probe.row, probe.start, probe.end), (0, 4, 17));
        // A column off any target has none.
        assert!(target_at(&screen, 0, 0).is_none());
    }

    #[test]
    fn target_at_finds_a_path_candidate_and_splits_its_position() {
        let screen = screen_from("at crates/pty/src/grid.rs:184 now");
        let probe = target_at(&screen, 10, 0).expect("column 10 is inside the path");
        assert_eq!(
            probe.kind,
            ProbeKind::Path {
                candidate: "crates/pty/src/grid.rs".into(),
                line: Some(184),
                col: None,
            }
        );
        // The whole run underlines, `:184` included — that is what was aimed at.
        assert_eq!((probe.start, probe.end), (3, 29));
    }

    #[test]
    fn a_url_wins_over_the_path_shape_inside_it() {
        // `https://ex.io/a/b` is path-shaped after its scheme. Opening it in an
        // editor is never what was meant.
        let screen = screen_from("see https://ex.io/a/b now");
        let probe = target_at(&screen, 15, 0).expect("column 15 is inside the URL");
        assert_eq!(probe.kind, ProbeKind::Url("https://ex.io/a/b".into()));
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
