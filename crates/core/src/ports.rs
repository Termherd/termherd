//! Ports — traits defining the boundary between the headless core and the
//! outside world. Adapters in sibling crates implement these.
//!
//! Signatures grow as adapters land (scan in M1, store in M1, pty in M2).
//! The dependency rule: `core` declares ports, never imports adapters.

use std::time::SystemTime;

use crate::app::SpawnSpec;
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
    /// Scroll a session's viewport by a line delta (positive = into history).
    fn scroll(&self, session: SessionId, delta: i32) -> Result<(), PtyError>;
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
