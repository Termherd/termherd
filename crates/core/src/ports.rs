//! Ports — traits defining the boundary between the headless core and the
//! outside world. Adapters in sibling crates implement these.
//!
//! Signatures grow as adapters land (scan in M1, store in M1, pty in M2).
//! The dependency rule: `core` declares ports, never imports adapters.

use std::time::SystemTime;

use crate::browser::SessionRecord;

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

/// Real signatures land with the `pty` adapter in M2.
pub trait PtyHost: Send + Sync {}
