//! The embedded terminal. Three concerns, one per file: [`canvas`] renders the
//! grid and wires pointer events (the `canvas::Program`); [`selection`] holds
//! the pure pointer/selection geometry; and this module owns the shared cell
//! metric ([`cell_size`]). The OS handoffs the shell performs for links and
//! notifications live in `shell/effects/os.rs`; the byte protocol and the grid
//! model live in `termherd_pty`.

mod canvas;
mod selection;

pub(super) use canvas::TerminalView;

/// Terminal cell metrics for the monospace grid, as ratios of the font size
/// so a zoomed font scales the grid proportionally. At the default
/// 14 px font they give the historical 8.4 × 18.0 cell. Used both to draw
/// and (in the parent) to translate the pane's pixel size into a PTY cell
/// geometry (FR4).
const CELL_W_RATIO: f32 = 8.4 / 14.0;
const CELL_H_RATIO: f32 = 18.0 / 14.0;

/// The cell box (width, height) for a terminal font size.
pub(super) fn cell_size(font_size: f32) -> (f32, f32) {
    (font_size * CELL_W_RATIO, font_size * CELL_H_RATIO)
}
