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
pub mod snapshot;
pub mod workspace;

pub use app::{
    App, DEFAULT_FONT_SIZE, Effect, Event, Launch, LaunchSpec, LiveSession, McpConfig,
    ScrollTarget, SelectOp, SelectSide, SessionStatus, SidebarFold, SpawnSpec, Zoom,
};
pub use browser::{ProjectGroup, SessionRecord};
pub use capture::{CaptureDump, CaptureTab};
pub use keymap::{Action, ActionBinding, ChordError, KeyChord, Keymap, action_catalog};
pub use metadata::{Overlay, RepoMeta, SessionMeta};
pub use record::Recording;
pub use snapshot::{
    ConfigInput, ConfigSummary, FocusRef, PaneSnapshot, ProjectSnapshot, Section, SessionKind,
    SidebarSnapshot, SnapshotFilter, SnapshotInputs, TabSnapshot, TerminalScope, WorkspaceSnapshot,
};
pub use workspace::{Branch, Pane, SessionId, SplitDir, Tab, Workspace};
