//! Record — the pure state machine for the GIF screencast (F-capture
//! rung 2).
//!
//! Clock-free by construction: the `app` runs the frame timer and feeds a
//! [`Event::RecordTick`](crate::app::Event::RecordTick) per frame; `core` only
//! counts frames against a cap and decides when to capture, finish, or cancel.
//! Frames are the time proxy — `max_frames = fps × max_seconds`, computed by the
//! app from settings — so the state machine needs no clock and is exhaustively
//! unit-testable. No I/O; encoding and the screenshot live in the `app` adapter.

/// An in-progress recording: frames captured so far and the cap that
/// auto-stops it. `frames` reaching `max_frames` ends the recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recording {
    /// Frames captured so far (one per [`RecordTick`](crate::app::Event::RecordTick)).
    pub frames: u32,
    /// Hard cap; the recording auto-stops once `frames` reaches it.
    pub max_frames: u32,
}
