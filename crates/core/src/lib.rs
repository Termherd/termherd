//! termherd-core — domain + headless `App` + workspace + keymap + ports.
//!
//! No I/O. No global mutable state. Pure, testable. See `docs/ARCHITECTURE.md`
//! §5 (headless core) and §6 (workspace/input model).

pub mod app;
pub mod browser;
pub mod docscope;
pub mod keymap;
pub mod links;
pub mod metadata;
pub mod ports;
pub mod workspace;

pub use app::{App, Effect, Event, LaunchSpec, LiveSession, SessionStatus, SpawnSpec};
pub use browser::{ProjectGroup, SessionRecord};
pub use keymap::{Action, ChordError, KeyChord, Keymap};
pub use metadata::SessionMeta;
pub use workspace::{Branch, Pane, SessionId, SplitDir, Tab, Workspace};
