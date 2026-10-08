//! termherd-core — domain + headless `App` + workspace + keymap + ports.
//!
//! No I/O. No global mutable state. Pure, testable. See `docs/ARCHITECTURE.md`
//! §5 (headless core) and §6 (workspace/input model).

pub mod app;
pub mod browser;
pub mod claude_command;
pub mod docscope;
pub mod keymap;
pub mod links;
pub mod metadata;
pub mod open;
pub mod paths;
pub mod ports;
pub mod record;
pub mod snapshot;
pub mod workspace;

pub use app::{
    App, ClaudeLaunch, CommandRefusal, DEFAULT_FONT_SIZE, Effect, Event, ForegroundJob,
    HoverTarget, Launch, LaunchSpec, LiveSession, McpConfig, MouseReporting, PathPurpose,
    PathRequest, PathRoots, PointerButton, PointerEvent, PointerKind, PointerRoute, ProbeKind,
    ResolvedPath, ScrollTarget, SelectOp, SelectSide, SessionStatus, SidebarFold, SpawnSpec,
    TargetProbe, TermHover, Zoom, claude_identity, grid_line, pointer_select,
};
pub use browser::{ProjectGroup, SessionRecord};
pub use claude_command::{ClaudeColor, ClaudeCommand, CommandArgument, CommandError};
pub use keymap::{Action, ActionBinding, ChordError, KeyChord, Keymap, action_catalog};
pub use metadata::{Overlay, RepoMeta, SessionMeta};
pub use open::{OpenCommand, OpenCommandError, OpenTarget};
pub use record::Recording;
pub use snapshot::{
    ClaudeIdentity, ConfigInput, ConfigSummary, FocusRef, PaneSnapshot, ProjectSnapshot, Section,
    SessionKind, SidebarSnapshot, SnapshotFilter, SnapshotInputs, TabSnapshot, TerminalScope,
    WorkspaceSnapshot,
};
pub use workspace::{Branch, Pane, SessionId, SplitDir, Tab, Workspace};
