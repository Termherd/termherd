//! termherd-core — domain + headless `App` + workspace + keymap + ports.
//!
//! No I/O. No global mutable state. Pure, testable. See `docs/ARCHITECTURE.md`
//! §5 (headless core) and §6 (workspace/input model).

pub mod app;
pub mod browser;
pub mod capture;
pub mod docscope;
pub mod keymap;
pub mod links;
pub mod metadata;
pub mod ports;
pub mod record;
pub mod workspace;

pub use app::{
    App, Effect, Event, Launch, LaunchSpec, LiveSession, ScrollTarget, SessionStatus, SidebarFold,
    SpawnSpec,
};
pub use browser::{ProjectGroup, SessionRecord};
pub use capture::{CaptureDump, CaptureTab};
pub use keymap::{Action, ChordError, KeyChord, Keymap};
pub use metadata::SessionMeta;
pub use record::Recording;
pub use workspace::{Branch, Pane, SessionId, SplitDir, Tab, Workspace};
