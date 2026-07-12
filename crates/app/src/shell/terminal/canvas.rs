//! The embedded terminal widget: a `canvas::Program` that draws the visible
//! grid + cursor (FR4), handles wheel scrollback and drag-to-select, and
//! resolves Ctrl/Cmd link hover/click. The byte protocol and the grid model
//! live in `termherd_pty`; the pure pointer/selection geometry lives in
//! [`super::selection`]; this is the rendering and pointer-event wiring.

use iced::advanced::mouse::{Click, click};
use iced::advanced::text::Shaping;
use iced::widget::canvas::{self, Frame, Geometry, Text};
use iced::{Color, Font, Pixels, Point, Rectangle, Renderer, Size, Theme, mouse};
use termherd_core::workspace::SessionId;
use termherd_pty::Screen;

use crate::shell::Message;

use super::cell_size;
use super::selection::{
    HoverLink, abs_line, cell_at, link_at, selection_text, visible_spans, word_at,
};

/// The terminal's default background (matches `termherd_pty`'s default).
const BG: Color = Color::from_rgb(
    0x11 as f32 / 255.0,
    0x13 as f32 / 255.0,
    0x18 as f32 / 255.0,
);

/// A canvas program that draws the visible terminal grid with per-cell colour
/// and the cursor (FR4), and handles wheel scrollback + drag-to-select.
/// The fields are `pub(in crate::shell)` because the view layer
/// (`crate::shell::view`) constructs it directly.
pub(in crate::shell) struct TerminalView<'a> {
    pub(in crate::shell) screen: &'a Screen,
    /// The session this canvas is currently showing. The canvas widget is
    /// reused across tabs, so the selection state is tagged with its owner to
    /// keep a selection from bleeding onto another tab.
    pub(in crate::shell) session: SessionId,
    /// Whether the link-open modifier (Ctrl/Cmd) is held, so a hovered link
    /// highlights and a click opens it instead of selecting text.
    pub(in crate::shell) link_modifier: bool,
    /// The effective terminal font size, from `core::App::font_size` —
    /// the glyph size, and (via [`cell_size`]) the wheel's line height.
    pub(in crate::shell) font_size: f32,
}

/// Per-canvas selection state: the drag in progress, the last range, and the
/// session it belongs to. The canvas widget is shared across tabs (iced keys
/// program state by tree position), so `owner` scopes the selection to one
/// session.
/// `pub(in crate::shell)` to match [`TerminalView`]: it is that widget's
/// `canvas::Program::State`, so it is as reachable as the widget itself.
#[derive(Default)]
pub(in crate::shell) struct TermState {
    selecting: bool,
    /// Selection endpoints as `(col, absolute scrollback line)` — see
    /// [`selection::abs_line`]. Anchoring to the absolute line (not a viewport
    /// row) lets the highlight follow the text through scroll.
    anchor: Option<(u16, i32)>,
    head: Option<(u16, i32)>,
    owner: Option<SessionId>,
    /// The link currently under the pointer while the modifier is held:
    /// its row, column span `[start, end)`, and the URL to open on click.
    hover: Option<HoverLink>,
    /// The last left-button press, kept so iced's click tracker can tell a
    /// double-click (select the word/filename under it) from a single one.
    last_click: Option<Click>,
    /// Banks fractional wheel deltas so fine-grained trackpad scrolls add up
    /// instead of rounding to zero.
    scroll: ScrollAccumulator,
}

/// Converts a wheel delta into a number of terminal lines. Mice send discrete
/// `Lines`; trackpads (notably macOS) send fine-grained `Pixels`, which we map
/// through the cell height. The result is fractional on purpose — banking the
/// fraction is the accumulator's job, not this one's (FR4).
fn delta_to_lines(delta: &mouse::ScrollDelta, cell_h: f32) -> f32 {
    match delta {
        mouse::ScrollDelta::Lines { y, .. } => *y,
        mouse::ScrollDelta::Pixels { y, .. } => y / cell_h,
    }
}

/// Banks the fractional part of successive wheel deltas so that fine-grained
/// trackpad scrolls aren't lost. macOS sends a stream of small pixel deltas
/// (a few px each); each one alone is a fraction of a cell and would round to
/// zero, so without banking the carry the terminal never scrolls.
#[derive(Default)]
pub(super) struct ScrollAccumulator {
    residual: f32,
}

impl ScrollAccumulator {
    /// Add `lines` to the carry and return the whole lines to scroll now,
    /// keeping the leftover fraction for next time. Banking the carry is what
    /// lets a run of sub-line trackpad deltas add up instead of each rounding
    /// to zero. By construction the residual stays within one line, so
    /// the emitted total never drifts from the true input.
    fn step(&mut self, lines: f32) -> i32 {
        self.residual += lines;
        let whole = self.residual.trunc();
        self.residual -= whole;
        whole as i32
    }
}

impl TermState {
    /// Drop any selection, keeping the owning session.
    fn clear_selection(&mut self) {
        self.selecting = false;
        self.anchor = None;
        self.head = None;
    }

    /// The current selection range, only when it spans more than one cell — a
    /// bare click (anchor == head) is not a selection.
    fn range(&self) -> Option<((u16, i32), (u16, i32))> {
        match (self.anchor, self.head) {
            (Some(a), Some(b)) if a != b => Some((a, b)),
            _ => None,
        }
    }
}

impl TerminalView<'_> {
    /// The highlighted spans to paint for the current selection: `(row, c0, c1)`
    /// with inclusive columns, one per visible row the selection covers. Empty
    /// unless this canvas owns a real (multi-cell) selection, so the highlight
    /// neither bleeds across tabs nor paints a bare click. This is the seam the
    /// draw pass and the selection tests share.
    fn visible_selection(&self, state: &TermState) -> Vec<(u16, u16, u16)> {
        if state.owner != Some(self.session) {
            return Vec::new();
        }
        let Some((a, b)) = state.range() else {
            return Vec::new();
        };
        visible_spans(
            a,
            b,
            self.screen.display_offset,
            self.screen.cols,
            self.screen.rows,
        )
    }
}

impl canvas::Program<Message> for TerminalView<'_> {
    type State = TermState;

    fn update(
        &self,
        state: &mut TermState,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let canvas::Event::Mouse(event) = event else {
            return None;
        };
        // The canvas is reused as tabs switch; if it now shows a different
        // session, the previous tab's selection must not carry over.
        if state.owner != Some(self.session) {
            *state = TermState {
                owner: Some(self.session),
                ..TermState::default()
            };
        }
        match event {
            // Wheel scrolls the viewport into scrollback history (FR4) — but
            // only when the pointer is actually over the terminal. The canvas
            // sees wheel events even while hovering the sidebar, so without
            // this guard scrolling the session list also scrolls the PTY.
            mouse::Event::WheelScrolled { delta } if cursor.position_in(bounds).is_some() => {
                // The selection is anchored to absolute scrollback lines, so it
                // rides with the text as the viewport moves — no need to drop it.
                let lines = delta_to_lines(delta, cell_size(self.font_size).1);
                let step = state.scroll.step(lines);
                // The pointer cell rides along so a mouse-mode app (Claude's TUI)
                // can be handed the wheel as input; the adapter falls back to our
                // scrollback when it isn't one. Computed only once a whole
                // line is banked, so sub-line trackpad ticks stay cheap.
                (step != 0).then(|| {
                    let (col, row) = cell_at(cursor, bounds, self.screen).unwrap_or((0, 0));
                    canvas::Action::publish(Message::TermScroll {
                        session: self.session,
                        col,
                        row,
                        lines: step,
                    })
                })
            }
            // Drag to select; the press is not captured so the wrapping
            // `mouse_area` still hands keyboard focus to the terminal.
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                let position = cursor.position_in(bounds)?;
                let (col, row) = cell_at(cursor, bounds, self.screen)?;
                // Ctrl/Cmd+click on a link opens it rather than selecting.
                if self.link_modifier
                    && let Some(link) = link_at(self.screen, col, row)
                {
                    return Some(canvas::Action::publish(Message::OpenUrl(link.url)));
                }
                // A double-click selects the whole word / filename under the
                // pointer and copies it, like a terminal. iced's click
                // tracker classifies the press from the previous one's time and
                // distance.
                let clicked = Click::new(position, mouse::Button::Left, state.last_click);
                state.last_click = Some(clicked);
                let offset = self.screen.display_offset;
                if clicked.kind() == click::Kind::Double
                    && let Some((anchor, head)) = word_at(self.screen, col, row)
                {
                    let (anchor, head) = (
                        (anchor.0, abs_line(anchor.1, offset)),
                        (head.0, abs_line(head.1, offset)),
                    );
                    state.selecting = false;
                    state.anchor = Some(anchor);
                    state.head = Some(head);
                    return Some(canvas::Action::publish(Message::CopySelection(
                        selection_text(self.screen, anchor, head),
                    )));
                }
                state.selecting = true;
                let cell = (col, abs_line(row, offset));
                state.anchor = Some(cell);
                state.head = Some(cell);
                Some(canvas::Action::request_redraw())
            }
            mouse::Event::CursorMoved { .. } if state.selecting => {
                cell_at(cursor, bounds, self.screen).map(|(col, row)| {
                    state.head = Some((col, abs_line(row, self.screen.display_offset)));
                    canvas::Action::request_redraw()
                })
            }
            // Track the link under the pointer while the modifier is held so the
            // draw pass can highlight it and the pointer turns into a hand.
            mouse::Event::CursorMoved { .. } => {
                let next = self
                    .link_modifier
                    .then(|| cell_at(cursor, bounds, self.screen))
                    .flatten()
                    .and_then(|(col, row)| link_at(self.screen, col, row));
                (next != state.hover).then(|| {
                    state.hover = next;
                    canvas::Action::request_redraw()
                })
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) if state.selecting => {
                state.selecting = false;
                // Only a real drag is a selection; a bare click clears it so a
                // single click can't leave an undismissable highlight.
                match state.range() {
                    Some((a, b)) => Some(canvas::Action::publish(Message::CopySelection(
                        selection_text(self.screen, a, b),
                    ))),
                    None => {
                        state.clear_selection();
                        Some(canvas::Action::request_redraw())
                    }
                }
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &TermState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let cols = self.screen.cols.max(1) as f32;
        let rows = self.screen.rows.max(1) as f32;
        let cell_w = bounds.width / cols;
        let cell_h = bounds.height / rows;

        frame.fill_rectangle(Point::ORIGIN, bounds.size(), BG);

        for (r, line) in self.screen.lines.iter().enumerate() {
            let y = r as f32 * cell_h;
            for (c, cell) in line.iter().enumerate() {
                let x = c as f32 * cell_w;
                if cell.bg != [0x11, 0x13, 0x18] {
                    frame.fill_rectangle(Point::new(x, y), Size::new(cell_w, cell_h), rgb(cell.bg));
                }
                if cell.c != ' ' && cell.c != '\0' {
                    frame.fill_text(Text {
                        content: cell.c.to_string(),
                        position: Point::new(x, y),
                        color: rgb(cell.fg),
                        size: Pixels(self.font_size),
                        font: Font::MONOSPACE,
                        shaping: Shaping::Advanced,
                        ..Text::default()
                    });
                }
            }
        }

        // Translucent overlay over the selected range, one rectangle per
        // visible row (the owner/real-selection gating lives in the seam).
        for (r, c0, c1) in self.visible_selection(state) {
            let x = c0 as f32 * cell_w;
            let w = (c1.saturating_sub(c0) + 1) as f32 * cell_w;
            frame.fill_rectangle(
                Point::new(x, r as f32 * cell_h),
                Size::new(w, cell_h),
                Color {
                    a: 0.3,
                    ..rgb([0x55, 0x88, 0xff])
                },
            );
        }

        // Underline the hovered link while the modifier is held, the classic
        // clickable-link affordance. Gated on the live modifier flag so
        // releasing Ctrl/Cmd clears the highlight even without a mouse move.
        if self.link_modifier
            && state.owner == Some(self.session)
            && let Some(link) = &state.hover
        {
            let x = link.start as f32 * cell_w;
            let w = (link.end.saturating_sub(link.start)) as f32 * cell_w;
            let y = (link.row as f32 + 1.0) * cell_h - 1.5;
            frame.fill_rectangle(Point::new(x, y), Size::new(w, 1.5), rgb([0x55, 0x88, 0xff]));
        }

        if let Some((cc, cr)) = self.screen.cursor {
            let x = cc as f32 * cell_w;
            let y = cr as f32 * cell_h;
            frame.fill_rectangle(
                Point::new(x, y),
                Size::new(cell_w, cell_h),
                Color {
                    a: 0.6,
                    ..rgb([0xd0, 0xd0, 0xd0])
                },
            );
        }

        vec![frame.into_geometry()]
    }

    /// A hand pointer over a hovered link with the modifier held, so the
    /// link is visibly clickable; otherwise the text/I-beam cursor while over
    /// the grid, signalling that the text is selectable; the default
    /// pointer when off the terminal entirely.
    fn mouse_interaction(
        &self,
        state: &TermState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.link_modifier && state.owner == Some(self.session) && state.hover.is_some() {
            mouse::Interaction::Pointer
        } else if cursor.position_in(bounds).is_some() {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::default()
        }
    }
}

fn rgb([r, g, b]: [u8; 3]) -> Color {
    Color::from_rgb8(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cell height at the default font — the tests' historical metric.
    const CELL_H: f32 = 18.0;

    fn sid(n: u64) -> SessionId {
        SessionId(std::num::NonZeroU64::new(n).expect("non-zero"))
    }

    /// A blank 4×2 screen; with 100×100 bounds each cell is 25×50 px, so
    /// (10,10) lands in cell (0,0) and (60,60) in cell (2,1).
    fn test_screen() -> Screen {
        use termherd_pty::ScreenCell;
        let cell = ScreenCell {
            c: ' ',
            fg: [0, 0, 0],
            bg: [0, 0, 0],
            bold: false,
        };
        Screen {
            cols: 4,
            rows: 2,
            lines: vec![vec![cell; 4]; 2],
            cursor: None,
            scrolled: false,
            display_offset: 0,
            bracketed_paste: false,
            selection: Vec::new(),
        }
    }

    fn test_bounds() -> Rectangle {
        Rectangle {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        }
    }

    fn at(x: f32, y: f32) -> mouse::Cursor {
        mouse::Cursor::Available(Point::new(x, y))
    }

    fn press() -> canvas::Event {
        canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
    }
    fn release() -> canvas::Event {
        canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
    }
    fn moved() -> canvas::Event {
        canvas::Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(60.0, 60.0),
        })
    }
    fn wheel() -> canvas::Event {
        canvas::Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
        })
    }

    #[test]
    fn wheel_scroll_only_acts_when_the_pointer_is_over_the_terminal() {
        use canvas::Program;
        let screen = test_screen();
        let view = TerminalView {
            screen: &screen,
            session: sid(1),
            link_modifier: false,
            font_size: 14.0,
        };
        // Pointer over the canvas → the scroll is published.
        let mut state = TermState::default();
        assert!(
            view.update(&mut state, &wheel(), test_bounds(), at(50.0, 50.0))
                .is_some()
        );
        // Pointer outside (e.g. over the sidebar) → ignored.
        let mut state = TermState::default();
        assert!(
            view.update(&mut state, &wheel(), test_bounds(), at(250.0, 50.0))
                .is_none()
        );
    }

    #[test]
    fn a_bare_click_leaves_no_selection() {
        // press and release on the same cell, no drag.
        use canvas::Program;
        let screen = test_screen();
        let view = TerminalView {
            screen: &screen,
            session: sid(1),
            link_modifier: false,
            font_size: 14.0,
        };
        let mut state = TermState::default();
        let _ = view.update(&mut state, &press(), test_bounds(), at(10.0, 10.0));
        let _ = view.update(&mut state, &release(), test_bounds(), at(10.0, 10.0));
        assert!(state.range().is_none(), "a click is not a selection");
        assert!(state.anchor.is_none() && state.head.is_none());
    }

    #[test]
    fn a_drag_makes_a_selection_and_copies() {
        use canvas::Program;
        let screen = test_screen();
        let view = TerminalView {
            screen: &screen,
            session: sid(1),
            link_modifier: false,
            font_size: 14.0,
        };
        let mut state = TermState::default();
        let _ = view.update(&mut state, &press(), test_bounds(), at(10.0, 10.0)); // (0,0)
        let _ = view.update(&mut state, &moved(), test_bounds(), at(60.0, 60.0)); // (2,1)
        assert!(state.range().is_some());
        // A real drag publishes a copy on release.
        assert!(
            view.update(&mut state, &release(), test_bounds(), at(60.0, 60.0))
                .is_some()
        );
    }

    #[test]
    fn selection_does_not_bleed_across_sessions() {
        // a selection on one session must not show for another.
        use canvas::Program;
        let screen = test_screen();
        let mut state = TermState::default();
        let s1 = TerminalView {
            screen: &screen,
            session: sid(1),
            link_modifier: false,
            font_size: 14.0,
        };
        let _ = s1.update(&mut state, &press(), test_bounds(), at(10.0, 10.0));
        let _ = s1.update(&mut state, &moved(), test_bounds(), at(60.0, 60.0));
        assert_eq!(state.owner, Some(sid(1)));
        assert!(state.range().is_some());
        // The canvas now shows session 2; its first event drops the stale one.
        let s2 = TerminalView {
            screen: &screen,
            session: sid(2),
            link_modifier: false,
            font_size: 14.0,
        };
        let _ = s2.update(&mut state, &release(), test_bounds(), at(60.0, 60.0));
        assert_eq!(state.owner, Some(sid(2)));
        assert!(
            state.range().is_none(),
            "selection must not carry to another session"
        );
    }

    // --- the selection follows the text through scroll ---------------------

    /// A blank 4×`rows` screen scrolled up `offset` lines into scrollback.
    fn screen_rows(rows: u16, offset: usize) -> Screen {
        use termherd_pty::ScreenCell;
        let cell = ScreenCell {
            c: ' ',
            fg: [0, 0, 0],
            bg: [0, 0, 0],
            bold: false,
        };
        Screen {
            cols: 4,
            rows,
            lines: vec![vec![cell; 4]; rows as usize],
            cursor: None,
            scrolled: offset > 0,
            display_offset: offset,
            bracketed_paste: false,
            selection: Vec::new(),
        }
    }

    /// A cursor over the centre of grid cell `(col, row)` on a 4×`rows` grid
    /// filling the 100×100 test bounds.
    fn at_cell(col: u16, row: u16, rows: u16) -> mouse::Cursor {
        let cw = 100.0 / 4.0;
        let ch = 100.0 / rows as f32;
        at((col as f32 + 0.5) * cw, (row as f32 + 0.5) * ch)
    }

    /// The viewport rows a set of highlighted spans covers.
    fn rows_of(spans: &[(u16, u16, u16)]) -> Vec<u16> {
        spans.iter().map(|s| s.0).collect()
    }

    fn view_of(screen: &Screen) -> TerminalView<'_> {
        TerminalView {
            screen,
            session: sid(1),
            link_modifier: false,
            font_size: 14.0,
        }
    }

    #[test]
    fn selection_survives_a_wheel_scroll() {
        // A wheel scroll must keep the selection, re-anchored to the text,
        // rather than dropping it (the old viewport-coordinate behaviour).
        use canvas::Program;
        let screen = test_screen();
        let view = view_of(&screen);
        let mut state = TermState::default();
        let _ = view.update(&mut state, &press(), test_bounds(), at(10.0, 10.0));
        let _ = view.update(&mut state, &moved(), test_bounds(), at(60.0, 60.0));
        assert!(state.range().is_some());
        let action = view.update(&mut state, &wheel(), test_bounds(), at(50.0, 50.0));
        assert!(
            state.range().is_some(),
            "a wheel scroll must keep the selection anchored to the text"
        );
        assert!(
            action.is_some(),
            "the wheel still drives the terminal scroll"
        );
    }

    #[test]
    fn the_highlight_follows_the_text_through_scroll() {
        // Select viewport row 1 at the live tail, then scroll up one line: the
        // same text is now one row lower, so the highlight must ride down to it.
        use canvas::Program;
        let screen0 = screen_rows(3, 0);
        let view0 = view_of(&screen0);
        let mut state = TermState::default();
        let _ = view0.update(&mut state, &press(), test_bounds(), at_cell(0, 1, 3));
        let _ = view0.update(&mut state, &moved(), test_bounds(), at_cell(3, 1, 3));
        assert_eq!(
            rows_of(&view0.visible_selection(&state)),
            vec![1],
            "highlighted at row 1 before scroll"
        );
        let screen1 = screen_rows(3, 1);
        let view1 = view_of(&screen1);
        assert_eq!(
            rows_of(&view1.visible_selection(&state)),
            vec![2],
            "the highlight rides down to row 2 with the text after scrolling up one line"
        );
    }

    #[test]
    fn a_selection_scrolled_off_the_top_is_clipped() {
        // Select while already scrolled up two lines, then return to the live
        // tail: the selected text is now above the viewport and its highlight
        // must clip out entirely rather than staying pinned to a row.
        use canvas::Program;
        let scrolled = screen_rows(3, 2);
        let view = view_of(&scrolled);
        let mut state = TermState::default();
        let _ = view.update(&mut state, &press(), test_bounds(), at_cell(0, 0, 3));
        let _ = view.update(&mut state, &moved(), test_bounds(), at_cell(3, 0, 3));
        assert_eq!(
            rows_of(&view.visible_selection(&state)),
            vec![0],
            "highlighted at row 0 while scrolled"
        );
        let tail = screen_rows(3, 0);
        let view_tail = view_of(&tail);
        assert!(
            view_tail.visible_selection(&state).is_empty(),
            "a selection scrolled off the top is clipped out of the viewport"
        );
    }

    /// A single-row screen holding `line`, one char per cell (link tests).
    fn screen_from(line: &str) -> Screen {
        use termherd_pty::ScreenCell;
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

    /// A cursor over the centre of column `col` on the single row, given a line
    /// of `len` chars filling the 100px-wide test bounds.
    fn at_col(len: usize, col: usize) -> mouse::Cursor {
        let cw = 100.0 / len as f32;
        at((col as f32 + 0.5) * cw, 50.0)
    }

    #[test]
    fn modifier_click_on_a_link_opens_instead_of_selecting() {
        // Ctrl/Cmd+click publishes an open and starts no selection.
        use canvas::Program;
        let screen = screen_from("https://ex.io");
        let len = "https://ex.io".len();
        let view = TerminalView {
            screen: &screen,
            session: sid(1),
            link_modifier: true,
            font_size: 14.0,
        };
        let mut state = TermState::default();
        let action = view.update(&mut state, &press(), test_bounds(), at_col(len, 2));
        assert!(action.is_some(), "a link click yields an action");
        assert!(!state.selecting && state.anchor.is_none());
    }

    #[test]
    fn modifier_click_off_a_link_still_selects() {
        // holding the modifier away from any link falls back to selection.
        use canvas::Program;
        let screen = screen_from("plain text only");
        let len = "plain text only".len();
        let view = TerminalView {
            screen: &screen,
            session: sid(1),
            link_modifier: true,
            font_size: 14.0,
        };
        let mut state = TermState::default();
        let _ = view.update(&mut state, &press(), test_bounds(), at_col(len, 2));
        assert!(state.selecting && state.anchor.is_some());
    }

    #[test]
    fn hover_highlights_a_link_only_with_the_modifier_held() {
        use canvas::Program;
        let screen = screen_from("https://ex.io");
        let len = "https://ex.io".len();
        // Modifier held → moving over the link records it for highlighting.
        let held = TerminalView {
            screen: &screen,
            session: sid(1),
            link_modifier: true,
            font_size: 14.0,
        };
        let mut state = TermState::default();
        let _ = held.update(&mut state, &moved(), test_bounds(), at_col(len, 2));
        assert_eq!(
            state.hover.as_ref().map(|h| h.url.as_str()),
            Some("https://ex.io")
        );
        // No modifier → no hovered link is tracked.
        let bare = TerminalView {
            screen: &screen,
            session: sid(1),
            link_modifier: false,
            font_size: 14.0,
        };
        let mut state = TermState::default();
        let _ = bare.update(&mut state, &moved(), test_bounds(), at_col(len, 2));
        assert!(state.hover.is_none());
    }

    #[test]
    fn double_click_selects_and_copies_the_word_under_the_pointer() {
        // two consecutive presses on the same cell select the whole
        // word/filename run and publish a copy — without leaving an active drag.
        use canvas::Program;
        let line = "see src/main.rs now";
        let screen = screen_from(line);
        let view = TerminalView {
            screen: &screen,
            session: sid(1),
            link_modifier: false,
            font_size: 14.0,
        };
        let mut state = TermState::default();
        let cursor = at_col(line.len(), 8); // inside `src/main.rs` (cols 4..=14)
        let _ = view.update(&mut state, &press(), test_bounds(), cursor);
        let action = view.update(&mut state, &press(), test_bounds(), cursor);
        assert_eq!(state.anchor, Some((4, 0)));
        assert_eq!(state.head, Some((14, 0)));
        assert!(
            !state.selecting,
            "a word selection is settled, not a live drag"
        );
        assert!(action.is_some(), "double-click publishes a copy");
    }

    #[test]
    fn double_click_on_a_blank_starts_a_plain_selection() {
        // with no word under the pointer the double-click falls back to the
        // ordinary press behaviour rather than selecting nothing oddly.
        use canvas::Program;
        let line = "ab   cd"; // cols 2,3,4 are blanks
        let screen = screen_from(line);
        let view = TerminalView {
            screen: &screen,
            session: sid(1),
            link_modifier: false,
            font_size: 14.0,
        };
        let mut state = TermState::default();
        let cursor = at_col(line.len(), 3);
        let _ = view.update(&mut state, &press(), test_bounds(), cursor);
        let _ = view.update(&mut state, &press(), test_bounds(), cursor);
        assert!(state.selecting, "a blank double-click is a normal press");
        assert_eq!(state.anchor, Some((3, 0)));
        assert_eq!(state.head, Some((3, 0)));
    }

    #[test]
    fn pointer_is_a_text_beam_over_the_grid_only() {
        // the I-beam signals selectable text while over the terminal; off
        // it (e.g. the cursor sits over the sidebar) the default pointer returns.
        use canvas::Program;
        let screen = test_screen();
        let view = TerminalView {
            screen: &screen,
            session: sid(1),
            link_modifier: false,
            font_size: 14.0,
        };
        let state = TermState::default();
        assert_eq!(
            view.mouse_interaction(&state, test_bounds(), at(50.0, 50.0)),
            mouse::Interaction::Text
        );
        assert_eq!(
            view.mouse_interaction(&state, test_bounds(), at(250.0, 50.0)),
            mouse::Interaction::default()
        );
    }

    // --- wheel scroll accumulation (macOS trackpad) ---------------------

    /// A pixel wheel delta of `px`, as macOS trackpads send.
    fn pixels(px: f32) -> mouse::ScrollDelta {
        mouse::ScrollDelta::Pixels { x: 0.0, y: px }
    }
    /// A discrete line wheel delta, as a mouse notch sends.
    fn lines(y: f32) -> mouse::ScrollDelta {
        mouse::ScrollDelta::Lines { x: 0.0, y }
    }

    #[test]
    fn whole_line_deltas_scroll_one_for_one() {
        // Regression guard: a mouse notch must keep scrolling exactly one line.
        let mut acc = ScrollAccumulator::default();
        assert_eq!(acc.step(delta_to_lines(&lines(1.0), CELL_H)), 1);
        assert_eq!(acc.step(delta_to_lines(&lines(3.0), CELL_H)), 3);
        assert_eq!(acc.step(delta_to_lines(&lines(-1.0), CELL_H)), -1);
    }

    #[test]
    fn small_trackpad_deltas_eventually_scroll_instead_of_vanishing() {
        // Each macOS pixel delta is a fraction of a cell (6/18 ≈ 0.33 line) and
        // rounds to zero alone; banked, a few of them must move one line.
        let mut acc = ScrollAccumulator::default();
        let one = delta_to_lines(&pixels(6.0), CELL_H); // ≈ 0.333 line
        let total: i32 = (0..4).map(|_| acc.step(one)).sum();
        assert!(
            total >= 1,
            "four 6px trackpad ticks must scroll at least one line, got {total}"
        );
    }

    #[test]
    fn no_scroll_is_lost_across_a_stream() {
        // A run of sub-line deltas totalling 2.6 lines must emit 2 lines now and
        // bank the 0.6 leftover — never silently drop the lot.
        let mut acc = ScrollAccumulator::default();
        let step = delta_to_lines(&pixels(CELL_H * 0.26), CELL_H); // 0.26 line
        let total: i32 = (0..10).map(|_| acc.step(step)).sum();
        assert_eq!(total, 2, "10 × 0.26 line = 2.6 → 2 lines emitted");
    }

    #[test]
    fn accumulation_is_direction_symmetric() {
        // Upward (negative) trackpad scroll banks exactly like downward.
        let mut acc = ScrollAccumulator::default();
        let up = delta_to_lines(&pixels(-6.0), CELL_H); // ≈ -0.333 line
        let total: i32 = (0..4).map(|_| acc.step(up)).sum();
        assert!(
            total <= -1,
            "four upward ticks must scroll at least one line up, got {total}"
        );
    }

    proptest::proptest! {
        /// Conservation: at every prefix of an arbitrary delta stream the lines
        /// emitted so far stay within one line of the true cumulative input —
        /// nothing is lost, nothing is invented.
        #[test]
        fn emitted_lines_never_drift_more_than_one_line(
            deltas in proptest::collection::vec(-5.0f32..5.0, 0..200)
        ) {
            let mut acc = ScrollAccumulator::default();
            // Reconstruct the true cumulative input in f64: the accumulator
            // banks its carry in f32, so an f32 running sum here drifts from it
            // by rounding noise that grows with the stream and can nudge the
            // bound just past 1.0 (observed ~1e-6). A small epsilon absorbs that
            // float noise without weakening the "within one line" invariant.
            const EPS: f64 = 1e-3;
            let mut input = 0.0f64;
            let mut emitted = 0i64;
            for d in deltas {
                input += f64::from(d);
                emitted += i64::from(acc.step(d));
                let drift = (input - emitted as f64).abs();
                proptest::prop_assert!(
                    drift < 1.0 + EPS,
                    "drift {drift} exceeds one line (input {input}, emitted {emitted})"
                );
            }
        }
    }
}
