//! Ports — traits defining the boundary between the headless core and the
//! outside world. Adapters in sibling crates implement these.
//!
//! Signatures grow as adapters land (scan in M1, store in M1, pty in M2).
//! The dependency rule: `core` declares ports, never imports adapters.

use std::path::PathBuf;
use std::time::SystemTime;

use crate::app::{PathRoots, ScrollTarget, SelectOp, SpawnSpec};
use crate::browser::SessionRecord;
use crate::workspace::SessionId;

pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
}

/// Discover sessions on disk. Implemented by `crates/scan` (M1).
pub trait ProjectScanner: Send + Sync {
    /// One full scan of the projects tree. Slow (filesystem) — run it off
    /// the UI thread (FR2); the result feeds `Event::ScanCompleted`.
    fn scan(&self) -> Result<Vec<SessionRecord>, ScanError>;
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ScanError {
    #[error("projects directory not readable: {0}")]
    Unreadable(String),
}

/// Turn a path-shaped run of terminal text into a file that actually exists.
/// Implemented by `crates/scan`, called when the pointer rests on a candidate
/// with the link modifier held, and again on the click.
///
/// **This is the filter that makes the feature usable.** `core::paths::detect`
/// is deliberately syntactic, so `and/or` and `http/2` reach here looking
/// exactly like `src/main.rs`; returning [`None`] for them is what keeps half
/// the screen from underlining. A candidate that resolves nowhere — prose, or a
/// real path on the far side of an ssh session — is simply not a link.
pub trait PathResolver: Send + Sync {
    /// The existing file `candidate` names, or [`None`].
    ///
    /// An absolute candidate is checked as-is. A relative one is tried against
    /// each root in [`PathRoots`] in order, most specific first: `cargo`, `git`
    /// and `pytest` each print relative to a different directory, so the same
    /// text can be meaningful from more than one — and the innermost match is
    /// the one the user meant.
    fn resolve(&self, candidate: &str, roots: &PathRoots) -> Option<PathBuf>;
}

/// Real signatures land with the `store` adapter in M1.
pub trait SessionStore: Send + Sync {}

/// Host the PTY processes behind the terminals. Implemented by `crates/pty`
/// (M2), called by the iced shell when it performs `core` effects. Output and
/// exit are delivered out-of-band (a sink given at construction, like the
/// scanner's watch callback), so this trait is only the control surface.
///
/// Each session is owned by its own task/thread inside the adapter; these
/// methods just message it. There is no shared `&mut Session` — the
/// structural fix for the `realSessionId` race (Q6).
pub trait PtyHost: Send + Sync {
    /// Spawn a PTY for an already-allocated session id.
    fn spawn(&self, spec: SpawnSpec) -> Result<(), PtyError>;
    /// Write bytes to a session's stdin.
    fn write(&self, session: SessionId, bytes: &[u8]) -> Result<(), PtyError>;
    /// Resize a session's PTY to the given cell geometry.
    fn resize(&self, session: SessionId, cols: u16, rows: u16) -> Result<(), PtyError>;
    /// Move a session's viewport: a relative line delta or an absolute jump to
    /// the top/bottom of the scrollback.
    fn scroll(&self, session: SessionId, target: ScrollTarget) -> Result<(), PtyError>;
    /// Apply a selection change to a session's terminal grid — anchored in the
    /// grid so the highlight follows the text through scroll.
    fn select(&self, session: SessionId, op: SelectOp) -> Result<(), PtyError>;
    /// Ask a session's terminal to copy its current selection; the text is
    /// delivered out-of-band via the event sink, read from the live selection.
    fn copy_selection(&self, session: SessionId) -> Result<(), PtyError>;
    /// Terminate a session's process and drop its task.
    fn kill(&self, session: SessionId) -> Result<(), PtyError>;
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum PtyError {
    #[error("no live session {0}")]
    NoSuchSession(u64),
    #[error("pty spawn failed: {0}")]
    Spawn(String),
    #[error("pty io failed: {0}")]
    Io(String),
}
