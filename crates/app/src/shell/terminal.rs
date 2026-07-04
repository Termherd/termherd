//! The embedded terminal widget: a `canvas::Program` that draws the visible
//! grid + cursor (FR4), handles wheel scrollback and drag-to-select, and
//! resolves Ctrl/Cmd link hover/click (#28). Plus the OS link opener. The byte
//! protocol and the grid model live in `termherd_pty`; this is pure rendering
//! and pointer logic.

use iced::advanced::mouse::{Click, click};
use iced::advanced::text::Shaping;
use iced::widget::canvas::{self, Frame, Geometry, Text};
use iced::{Color, Font, Pixels, Point, Rectangle, Renderer, Size, Theme, mouse};
use termherd_core::workspace::SessionId;
use termherd_pty::Screen;

use super::Message;

/// Terminal cell metrics for the monospace grid, as ratios of the font size
/// so a zoomed font scales the grid proportionally (#35). At the default
/// 14 px font they give the historical 8.4 × 18.0 cell. Used both to draw
/// and (in the parent) to translate the pane's pixel size into a PTY cell
/// geometry (FR4).
const CELL_W_RATIO: f32 = 8.4 / 14.0;
const CELL_H_RATIO: f32 = 18.0 / 14.0;

/// The cell box (width, height) for a terminal font size (#35).
pub(super) fn cell_size(font_size: f32) -> (f32, f32) {
    (font_size * CELL_W_RATIO, font_size * CELL_H_RATIO)
}
/// The terminal's default background (matches `termherd_pty`'s default).
const BG: Color = Color::from_rgb(
    0x11 as f32 / 255.0,
    0x13 as f32 / 255.0,
    0x18 as f32 / 255.0,
);

/// A canvas program that draws the visible terminal grid with per-cell colour
/// and the cursor (FR4), and handles wheel scrollback + drag-to-select.
pub(super) struct TerminalView<'a> {
    pub(super) screen: &'a Screen,
    /// The session this canvas is currently showing. The canvas widget is
    /// reused across tabs, so the selection state is tagged with its owner to
    /// keep a selection from bleeding onto another tab (#7).
    pub(super) session: SessionId,
    /// Whether the link-open modifier (Ctrl/Cmd) is held, so a hovered link
    /// highlights and a click opens it instead of selecting text (#28).
    pub(super) link_modifier: bool,
    /// The effective terminal font size (#35), from `core::App::font_size` —
    /// the glyph size, and (via [`cell_size`]) the wheel's line height.
    pub(super) font_size: f32,
}

/// Per-canvas selection state: the drag in progress, the last range, and the
/// session it belongs to. The canvas widget is shared across tabs (iced keys
/// program state by tree position), so `owner` scopes the selection to one
/// session (#7).
#[derive(Default)]
pub(super) struct TermState {
    selecting: bool,
    anchor: Option<(u16, u16)>,
    head: Option<(u16, u16)>,
    owner: Option<SessionId>,
    /// The link currently under the pointer while the modifier is held (#28):
    /// its row, column span `[start, end)`, and the URL to open on click.
    hover: Option<HoverLink>,
    /// The last left-button press, kept so iced's click tracker can tell a
    /// double-click (select the word/filename under it) from a single one (#27).
    last_click: Option<Click>,
    /// Banks fractional wheel deltas so fine-grained trackpad scrolls add up
    /// instead of rounding to zero (#98).
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
/// zero, so without banking the carry the terminal never scrolls (#98).
#[derive(Default)]
pub(super) struct ScrollAccumulator {
    residual: f32,
}

impl ScrollAccumulator {
    /// Add `lines` to the carry and return the whole lines to scroll now,
    /// keeping the leftover fraction for next time. Banking the carry is what
    /// lets a run of sub-line trackpad deltas add up instead of each rounding
    /// to zero (#98). By construction the residual stays within one line, so
    /// the emitted total never drifts from the true input.
    fn step(&mut self, lines: f32) -> i32 {
        self.residual += lines;
        let whole = self.residual.trunc();
        self.residual -= whole;
        whole as i32
    }
}

/// A link the pointer is hovering with Ctrl/Cmd held — what to highlight and,
/// on click, what to open (#28).
#[derive(Clone, PartialEq, Eq)]
struct HoverLink {
    row: u16,
    start: u16,
    end: u16,
    url: String,
}

impl TermState {
    /// Drop any selection, keeping the owning session.
    fn clear_selection(&mut self) {
        self.selecting = false;
        self.anchor = None;
        self.head = None;
    }

    /// The current selection range, only when it spans more than one cell — a
    /// bare click (anchor == head) is not a selection (#6).
    fn range(&self) -> Option<((u16, u16), (u16, u16))> {
        match (self.anchor, self.head) {
            (Some(a), Some(b)) if a != b => Some((a, b)),
            _ => None,
        }
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
        // session, the previous tab's selection must not carry over (#7).
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
            // this guard scrolling the session list also scrolls the PTY (#5).
            mouse::Event::WheelScrolled { delta } if cursor.position_in(bounds).is_some() => {
                // The selection is in viewport coordinates, so scrolling would
                // leave it floating over the wrong text; drop it (#8).
                state.clear_selection();
                let lines = delta_to_lines(delta, cell_size(self.font_size).1);
                let step = state.scroll.step(lines);
                // The pointer cell rides along so a mouse-mode app (Claude's TUI)
                // can be handed the wheel as input; the adapter falls back to our
                // scrollback when it isn't one (#98). Computed only once a whole
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
                // Ctrl/Cmd+click on a link opens it rather than selecting (#28).
                if self.link_modifier
                    && let Some(link) = link_at(self.screen, col, row)
                {
                    return Some(canvas::Action::publish(Message::OpenUrl(link.url)));
                }
                // A double-click selects the whole word / filename under the
                // pointer and copies it, like a terminal (#27). iced's click
                // tracker classifies the press from the previous one's time and
                // distance.
                let clicked = Click::new(position, mouse::Button::Left, state.last_click);
                state.last_click = Some(clicked);
                if clicked.kind() == click::Kind::Double
                    && let Some((anchor, head)) = word_at(self.screen, col, row)
                {
                    state.selecting = false;
                    state.anchor = Some(anchor);
                    state.head = Some(head);
                    return Some(canvas::Action::publish(Message::CopySelection(
                        selection_text(self.screen, anchor, head),
                    )));
                }
                state.selecting = true;
                state.anchor = Some((col, row));
                state.head = Some((col, row));
                Some(canvas::Action::request_redraw())
            }
            mouse::Event::CursorMoved { .. } if state.selecting => {
                cell_at(cursor, bounds, self.screen).map(|cell| {
                    state.head = Some(cell);
                    canvas::Action::request_redraw()
                })
            }
            // Track the link under the pointer while the modifier is held so the
            // draw pass can highlight it and the pointer turns into a hand (#28).
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
                // single click can't leave an undismissable highlight (#6).
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

        // Translucent overlay over the selected range — only the owning
        // session's real (multi-cell) selection, so it neither bleeds across
        // tabs (#7) nor paints a bare click (#6).
        if let (Some((a, b)), true) = (state.range(), state.owner == Some(self.session)) {
            let (start, end) = ordered(a, b);
            for r in start.1..=end.1 {
                let (c0, c1) = selection_span(start, end, r, self.screen.cols);
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
        }

        // Underline the hovered link while the modifier is held, the classic
        // clickable-link affordance (#28). Gated on the live modifier flag so
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

    /// A hand pointer over a hovered link with the modifier held (#28), so the
    /// link is visibly clickable; otherwise the text/I-beam cursor while over
    /// the grid, signalling that the text is selectable (#27); the default
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

/// The grid cell under the cursor, if any.
fn cell_at(cursor: mouse::Cursor, bounds: Rectangle, screen: &Screen) -> Option<(u16, u16)> {
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

/// The link under grid cell `(col, row)`, if any (#28). Builds the row's text
/// from its cells — one char per cell, so a `core::links` char-index span maps
/// straight onto columns — and returns the span containing `col`.
fn link_at(screen: &Screen, col: u16, row: u16) -> Option<HoverLink> {
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

/// Whether a character belongs to a double-click "word" (#27). Alphanumerics
/// plus the punctuation that holds filenames and paths together, so a unit like
/// `~/src/main.rs:42` selects whole; whitespace and bracketing punctuation
/// (quotes, parens, commas) are boundaries.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '\\' | '~' | ':' | '@' | '+')
}

/// The word / filename under grid cell `(col, row)` as an inclusive cell range
/// `(anchor, head)`, or `None` when the cell is not part of a word (e.g. blank).
/// A word is the maximal run of [`is_word_char`] cells around `col` — this is
/// what a double-click selects (#27).
fn word_at(screen: &Screen, col: u16, row: u16) -> Option<((u16, u16), (u16, u16))> {
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
fn ordered(a: (u16, u16), b: (u16, u16)) -> ((u16, u16), (u16, u16)) {
    if (a.1, a.0) <= (b.1, b.0) {
        (a, b)
    } else {
        (b, a)
    }
}

/// The selected column span `[c0, c1]` on row `r` of an ordered selection.
fn selection_span(start: (u16, u16), end: (u16, u16), r: u16, cols: u16) -> (u16, u16) {
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
fn selection_text(screen: &Screen, a: (u16, u16), b: (u16, u16)) -> String {
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

/// Hand a detected link to the OS default handler (#28). Fire-and-forget: the
/// child opener is spawned, not waited on. `url` has already been validated by
/// `core` (a recognised scheme, trimmed), and is always passed as a single
/// argument — never through a shell — so it can't be reinterpreted.
pub(super) fn open_url(url: &str) -> Result<(), termherd_core::ports::PtyError> {
    use std::process::Command;
    let spawn = |mut cmd: Command| {
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| termherd_core::ports::PtyError::Io(e.to_string()))
    };
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("open");
        cmd.arg(url);
        spawn(cmd)
    }
    #[cfg(target_os = "windows")]
    {
        // `start` treats the first quoted argument as the window title, so the
        // empty "" keeps the URL from being swallowed as one.
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "", url]);
        spawn(cmd)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(url);
        spawn(cmd)
    }
}

/// macOS bundle identifier (matches `Cargo.toml`'s packager `identifier`).
/// Used to attribute desktop notifications to TermHerd; see [`notify`].
#[cfg(target_os = "macos")]
const MACOS_BUNDLE_ID: &str = "dev.termherd";

/// Post a desktop notification to the OS notification centre (#29). Like
/// `open_url`, this is an OS handoff, not a PTY call, and fire-and-forget: the
/// send runs on a detached thread and the result is logged there, never fatal —
/// a notification backend that's unavailable must not take a session down.
/// `title`/`body` come pre-derived from `core` (which session, what message).
///
/// **Why a thread, not a direct call:** on macOS the backend (`NSUserNotification`
/// via `mac-notification-sys`) drives an `NSRunLoop` to await delivery *when
/// invoked on the main thread*. iced calls `perform` from inside winit's event
/// handler, so pumping the run loop there re-enters it and aborts the process.
/// Off the main thread the backend takes a Condvar wait instead, so this is
/// both crash-safe and non-blocking for the UI.
pub(super) fn notify(title: &str, body: &str) -> Result<(), termherd_core::ports::PtyError> {
    // Attribute notifications to our bundle once, before the first send, so the
    // macOS backend doesn't AppleScript-probe for a placeholder app and pop a
    // "Where is …?" chooser. No-op (and harmless) when run unbundled.
    #[cfg(target_os = "macos")]
    {
        use std::sync::Once;
        static SET_APP: Once = Once::new();
        SET_APP.call_once(|| {
            let _ = notify_rust::set_application(MACOS_BUNDLE_ID);
        });
    }

    let (title, body) = (title.to_owned(), body.to_owned());
    std::thread::Builder::new()
        .name("os-notify".to_owned())
        .spawn(move || {
            if let Err(error) = notify_rust::Notification::new()
                .summary(&title)
                .body(&body)
                .show()
            {
                tracing::warn!(%error, "desktop notification failed");
            }
        })
        .map(|_| ())
        .map_err(|e| termherd_core::ports::PtyError::Io(e.to_string()))
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
            bracketed_paste: false,
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
        // Pointer outside (e.g. over the sidebar) → ignored (#5).
        let mut state = TermState::default();
        assert!(
            view.update(&mut state, &wheel(), test_bounds(), at(250.0, 50.0))
                .is_none()
        );
    }

    #[test]
    fn a_bare_click_leaves_no_selection() {
        // #6: press and release on the same cell, no drag.
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
        // #7: a selection on one session must not show for another.
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

    #[test]
    fn scrolling_clears_the_selection() {
        // #8: a viewport-relative selection is dropped on scroll.
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
        let _ = view.update(&mut state, &moved(), test_bounds(), at(60.0, 60.0));
        assert!(state.range().is_some());
        let _ = view.update(&mut state, &wheel(), test_bounds(), at(50.0, 50.0));
        assert!(state.range().is_none(), "scroll must clear the selection");
    }

    /// A single-row screen holding `line`, one char per cell (#28 link tests).
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
            bracketed_paste: false,
        }
    }

    /// A cursor over the centre of column `col` on the single row, given a line
    /// of `len` chars filling the 100px-wide test bounds.
    fn at_col(len: usize, col: usize) -> mouse::Cursor {
        let cw = 100.0 / len as f32;
        at((col as f32 + 0.5) * cw, 50.0)
    }

    #[test]
    fn link_at_finds_the_url_under_a_column() {
        // #28: the column maps onto the detected span and yields its URL.
        let screen = screen_from("see https://ex.io now");
        let link = link_at(&screen, 6, 0).expect("column 6 is inside the URL");
        assert_eq!(link.url, "https://ex.io");
        assert_eq!((link.start, link.end), (4, 17));
        // A column off the URL has no link.
        assert!(link_at(&screen, 0, 0).is_none());
    }

    #[test]
    fn modifier_click_on_a_link_opens_instead_of_selecting() {
        // #28: Ctrl/Cmd+click publishes an open and starts no selection.
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
        // #28: holding the modifier away from any link falls back to selection.
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
    fn word_at_spans_a_filename_run() {
        // #27: a path/filename is one word — letters, digits and the joining
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
    fn double_click_selects_and_copies_the_word_under_the_pointer() {
        // #27: two consecutive presses on the same cell select the whole
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
        // #27: with no word under the pointer the double-click falls back to the
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
        // #27: the I-beam signals selectable text while over the terminal; off
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

    // --- #98: wheel scroll accumulation (macOS trackpad) ---------------------

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
        // rounds to zero alone; banked, a few of them must move one line (#98).
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
        /// nothing is lost, nothing is invented (#98).
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
