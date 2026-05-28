//! agentmux-core — domain + headless `App` + workspace + keymap + ports.
//!
//! No I/O. No global mutable state. Pure, testable. See `docs/ARCHITECTURE.md`
//! §5 (headless core) and §6 (workspace/input model).

pub mod app;
pub mod keymap;
pub mod ports;
pub mod workspace;

pub use app::{App, Effect, Event};
pub use workspace::{Branch, Pane, SessionId, SplitDir, Tab, Workspace};
