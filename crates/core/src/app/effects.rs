//! The `Effect` enum — every side effect the runtime performs on the
//! headless [`App`](super::App)'s behalf.
//!
//! Kept unified alongside [`Event`](super::Event): the iced shell turns these
//! into `pty`-adapter calls (`docs/ARCHITECTURE.md` §8).

use std::collections::HashSet;

use crate::capture::CaptureDump;
use crate::metadata::Overlay;
use crate::workspace::SessionId;

use super::{ScrollTarget, SelectOp, SpawnSpec};

/// Side effects the runtime must perform. The iced shell turns these into
/// `pty`-adapter calls (`docs/ARCHITECTURE.md` §8).
#[derive(Debug, Clone)]
pub enum Effect {
    /// Spawn a PTY for a freshly launched session.
    Spawn(SpawnSpec),
    /// Write bytes to a session's PTY stdin.
    Write { session: SessionId, bytes: Vec<u8> },
    /// Resize a session's PTY to the given cell geometry.
    Resize {
        session: SessionId,
        cols: u16,
        rows: u16,
    },
    /// Move a session's viewport: a relative line delta or an absolute jump to
    /// the top/bottom of the scrollback.
    Scroll {
        session: SessionId,
        target: ScrollTarget,
    },
    /// Apply a selection change to a session's terminal grid.
    Select { session: SessionId, op: SelectOp },
    /// Ask a session's terminal to copy its current selection — the text comes
    /// back out-of-band (a PTY event), so it is read from the live selection.
    CopyTerminalSelection { session: SessionId },
    /// Terminate a session's PTY process.
    Kill(SessionId),
    /// Persist the whole metadata overlay (sessions + repos) as one file.
    SaveMetadata(Overlay),
    /// Persist the folded-project set.
    SaveCollapsed(HashSet<String>),
    /// Open a URL in the OS default handler; the shell performs it.
    OpenUrl(String),
    /// Post a desktop notification to the OS notification centre. The
    /// shell performs it; `title` names the session/project that wants the
    /// user, `body` is Claude's message.
    Notify { title: String, body: String },
    /// Write a captured state snapshot for the AI dev loop (G1). The shell
    /// encodes it to `capture-<ts>.json` and takes the companion PNG; `core`
    /// only builds the pure, diffable payload.
    Capture(CaptureDump),
    /// Begin a GIF screencast: the app opens the encoder and starts its
    /// frame timer. `core` has already entered the recording state.
    StartRecording,
    /// Capture one screencast frame: the app screenshots the window and
    /// appends it to the open encoder.
    CaptureFrame,
    /// Finalise the GIF screencast: the app flushes the encoder and writes
    /// `capture-<ts>.gif`. `capped` names *why* it stopped — `true` when the frame
    /// cap auto-stopped it, `false` on a manual ⌘⇧R stop — so the app logs the
    /// reason from `core`'s decision rather than re-deriving it from the effect mix.
    FinishRecording { capped: bool },
    /// Abandon a screencast that captured no frames: the app drops the
    /// encoder without writing a file. Emitted when a recording is stopped before
    /// the first frame.
    CancelRecording,
}
