//! The iced shell — intentionally thin (ARCHITECTURE §8): translate GUI
//! messages into `core` events, perform the returned `core` effects against
//! the adapters, and render `core` state.
//!
//! This module is the state-transition half — the `Shell` struct, the
//! `Message` enum, `update`/`subscription` and the command methods. The rest
//! is split by concern into submodules:
//!
//! - [`view`] — how state is rendered (sidebar, main pane, tabs).
//! - [`terminal`] — the embedded terminal `canvas::Program` + link opener.
//! - [`ime`] — the input-method wrapper that composes dead/accent keys.
//! - [`input`] — keyboard translation (chords / `TermKey` / modifiers).
//! - [`streams`] — the PTY-output and fs-watch subscription sources.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use iced::advanced::widget::{self, operate, operation::focusable};
use iced::futures::channel::mpsc::UnboundedReceiver;
use iced::widget::text_editor;
use iced::{Point, Size, Subscription, Task, Theme, keyboard, window};
use termherd_core::ports::{ProjectScanner, PtyHost};
use termherd_core::workspace::SessionId;
use termherd_core::{
    Action, CaptureDump, Effect, Keymap, Launch, LaunchSpec, Overlay, ScrollTarget, SessionRecord,
    SessionStatus,
};
use termherd_pty::{PtyEvent, Screen, TermKey};

use crate::docs::DocEntry;
use crate::record::{FrameStats, FrameThrottle, RecordConfig, Recorder};
use crate::settings::{CloseSettings, ThemeChoice};
use crate::window_config::WindowConfig;

mod ime;
mod input;
mod streams;
mod terminal;
mod view;

use input::{chord_of, event_modifiers, key_mods, numpad_char, to_term_key};
use streams::{PtyOutput, pty_stream, watch_stream};
use termherd_core::browser::project_label;
use terminal::{cell_size, notify, open_url};

/// Sidebar width and the chrome reserved around the terminal, in logical px.
/// Combined with the zoom-derived cell metrics ([`terminal::cell_size`])
/// to size the
/// PTY grid to the window (FR4 resize).
const SIDEBAR_W: f32 = 300.0;
/// Width the collapsed sidebar still occupies: just the slim "▶" handle.
/// The grid reserves this instead of `SIDEBAR_W` when hidden, so the reclaimed
/// space becomes columns rather than stretched cells. The view pins the
/// handle to exactly this width (`view::view`), so it is a contract the layout
/// honours, not an estimate that can silently drift.
pub(super) const HANDLE_W: f32 = 28.0;
const H_CHROME: f32 = 40.0;
const V_CHROME: f32 = 84.0;

fn search_id() -> widget::Id {
    widget::Id::new("termherd-search")
}

fn rename_id() -> widget::Id {
    widget::Id::new("termherd-rename")
}

fn tab_rename_id() -> widget::Id {
    widget::Id::new("termherd-tab-rename")
}

/// The user's home directory, the fallback cwd for "new shell here" when no
/// session is open to inherit one from. Falls back to "." if neither
/// `USERPROFILE` (Windows) nor `HOME` (Unix) is set, so a launch always has a
/// directory to start in.
fn home_dir() -> String {
    crate::paths::home_dir()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string())
}

/// Resolved user configuration handed to the shell at startup: the theme,
/// keymap and metadata overlay built from `settings.json` / `metadata.json`.
/// Bundled so the composition root passes one value, not a long argument list.
pub struct Startup {
    pub theme: ThemeChoice,
    pub keymap: Keymap,
    pub metadata: Overlay,
    /// Folded project paths restored from disk.
    pub collapsed: HashSet<String>,
    /// GIF screencast budget from settings.
    pub record: RecordConfig,
    /// Sidebar session limit from settings; `0` shows every session.
    pub session_limit: usize,
    /// Terminal base font size from settings.
    pub font_size: f32,
    /// Close-confirmation policy for tab close and app quit.
    pub close: CloseSettings,
}

pub fn run(
    scanner: Arc<dyn ProjectScanner>,
    watch_root: Option<PathBuf>,
    pty: Arc<dyn PtyHost>,
    pty_rx: UnboundedReceiver<PtyEvent>,
    startup: Startup,
) -> iced::Result {
    // Restore the saved bounds, but discard a position that now lands off every
    // connected monitor (e.g. a second screen that has since been unplugged), so
    // the window can't open out of reach.
    let config =
        WindowConfig::load().with_onscreen_position(&crate::window_config::current_screens());
    let position = match (config.x, config.y) {
        (Some(x), Some(y)) => window::Position::Specific(Point::new(x, y)),
        _ => window::Position::Centered,
    };
    let pty_output = PtyOutput::new(pty_rx);
    iced::application(
        move || {
            let mut shell = Shell::new(
                config,
                scanner.clone(),
                watch_root.clone(),
                pty.clone(),
                pty_output.clone(),
                Startup {
                    theme: startup.theme,
                    keymap: startup.keymap.clone(),
                    metadata: startup.metadata.clone(),
                    collapsed: startup.collapsed.clone(),
                    record: startup.record,
                    session_limit: startup.session_limit,
                    font_size: startup.font_size,
                    close: startup.close,
                },
            );
            let initial_scan = shell.rescan();
            (shell, initial_scan)
        },
        Shell::update,
        Shell::view,
    )
    .title(|_: &Shell| String::from("TermHerd"))
    .theme(Shell::theme)
    .window(window::Settings {
        size: Size::new(config.width, config.height),
        position,
        min_size: Some(Size::new(480.0, 320.0)),
        icon: window_icon(),
        ..window::Settings::default()
    })
    // Close requests are intercepted so bounds can be saved first.
    .exit_on_close_request(false)
    .subscription(Shell::subscription)
    .run()
}

/// The window icon (taskbar + title bar) decoded from the bundled PNG. iced
/// 0.14 only takes raw RGBA, so we decode the 256×256 icon here. `None` if it
/// can't be decoded — a missing icon must never block startup.
fn window_icon() -> Option<window::Icon> {
    let png = include_bytes!("../icons/256x256.png");
    let mut reader = png::Decoder::new(png.as_slice()).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    // The bundled icon is 8-bit RGBA; bail rather than ship a garbled image if
    // that ever changes underfoot.
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    window::icon::from_rgba(buf, info.width, info.height).ok()
}

/// Where keyboard input goes. The terminal is the default target once one is
/// open; clicking the search box hands keys to it instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Terminal,
    Search,
}

/// A plan / memory document open in the main pane (F-plans-memory). Holds the
/// editable buffer plus the state the save path needs: where it lives, whether
/// it is in the writable scope, and the mtime captured at load for the
/// concurrency guard.
struct OpenDoc {
    /// Sidebar label, shown in the editor header.
    label: String,
    /// File on disk; the scope predicate and save are measured against it.
    path: PathBuf,
    /// The editable text buffer (iced text editor state).
    content: text_editor::Content,
    /// mtime captured at load; the baseline for the concurrent-write guard.
    /// `None` if it could not be read (then no conflict can be detected).
    loaded_mtime: Option<SystemTime>,
    /// Whether the write-scope predicate permits saving this path.
    writable: bool,
    /// Unsaved edits since load or the last successful save.
    dirty: bool,
    /// Transient feedback after a save attempt.
    feedback: Option<DocFeedback>,
}

/// The outcome of the last save attempt, surfaced in the editor header.
enum DocFeedback {
    Saved,
    Error(String),
}

struct Shell {
    /// The headless core; all browser and session state lives there.
    core: termherd_core::App,
    bounds: WindowConfig,
    scanner: Arc<dyn ProjectScanner>,
    watch_root: Option<PathBuf>,
    scan_error: Option<String>,
    /// The PTY host adapter; effects from `core` are performed against it.
    pty: Arc<dyn PtyHost>,
    /// Streams PTY output/exit into the subscription (taken once).
    pty_output: PtyOutput,
    /// Latest rendered grid per session.
    screens: HashMap<SessionId, Screen>,
    /// Current keyboard target.
    focus: Focus,
    /// Last non-empty terminal selection, for the keyboard copy shortcut (FR4).
    selection: Option<String>,
    /// GUI chrome theme (FR10).
    theme: Theme,
    /// Configurable shortcut bindings (FR9).
    keymap: Keymap,
    /// In-progress inline rename: `(session id, edit buffer)` (F-session-metadata).
    renaming: Option<(String, String)>,
    /// In-progress inline tab rename: `(anchor session, edit buffer)`. Distinct
    /// from [`Self::renaming`] (a browsed session's title) — this overrides a
    /// tab's *display* title, and its dismissal commits on blur rather than
    /// cancelling. Anchored on the tab's first session (a stable handle) rather
    /// than a positional index, so a reorder or a sibling close can't retarget
    /// the pending edit at the wrong tab.
    tab_rename: Option<(SessionId, String)>,
    /// Browsable plan / memory docs (F-plans-memory), refreshed on scan.
    docs: Vec<DocEntry>,
    /// Whether a scan is currently in flight. At most one runs at a time;
    /// see [`Shell::rescan`].
    scan_in_flight: bool,
    /// A change arrived while a scan was in flight — run one follow-up scan
    /// when it settles. Any number of mid-scan bursts coalesce into this
    /// single bit, so a busy projects tree can't queue unbounded scans.
    rescan_pending: bool,
    /// The doc currently open in the main pane for viewing/editing, if any.
    open_doc: Option<OpenDoc>,
    /// A close awaiting confirmation: the tab index to kill, or `None`.
    /// Killing a session is destructive, so the close button arms this and a
    /// confirmation bar must be accepted before the PTY is actually killed —
    /// unless [`Self::close_confirm`] waives the prompt for this close.
    closing: Option<usize>,
    /// Whether tab close and app quit prompt first (from `settings.json`).
    close_confirm: CloseSettings,
    /// An archive awaiting confirmation: the session id to archive, or `None`.
    /// Archiving is easy to trigger by accident, so the archive button
    /// arms this and a confirmation bar must be accepted first. Un-archiving is
    /// harmless and stays a one-click action.
    archiving: Option<String>,
    /// A window close awaiting confirmation: the window id to close once the
    /// user accepts, or `None`. Quitting hard-kills every live session's Claude
    /// process (TerminateProcess / SIGKILL, no graceful shutdown), so a quit
    /// with sessions still running arms this modal first.
    closing_window: Option<window::Id>,
    /// Whether Ctrl (or Cmd) is currently held — the link-open modifier.
    /// Tracked from keyboard events and handed to the terminal canvas so it can
    /// highlight a hovered link and open it on click.
    link_modifier: bool,
    /// An in-progress tab drag (FR5 reorder): the tab being dragged and
    /// the slot the pointer is currently over. `None` when no drag is active.
    /// Transient pointer state only — the tab order itself lives in `core`.
    tab_drag: Option<TabDrag>,
    /// Set once the quit path has asked the iced runtime to terminate.
    /// The observable proof that quitting reached `iced::exit` — closing the
    /// only window is *not* enough on macOS (winit cancels the OS terminate and
    /// `exit_on_close_request(false)` keeps the runtime alive), so the process
    /// would otherwise survive Cmd+Q and hold the single-instance lock.
    exiting: bool,
    /// The GIF screencast budget: fps / duration cap / frame scale.
    /// Default for now; `settings.json` configurability is a follow-up.
    record_config: RecordConfig,
    /// The recorder thread for an in-progress screencast, or `None`. The
    /// encoder lives off the UI thread; the shell only feeds it frames.
    recorder: Option<Recorder>,
    /// Frame screenshots requested but not yet handed to the recorder.
    /// A stop waits for this to drain so the final frames aren't lost.
    record_inflight: u32,
    /// A finish is pending until the last in-flight frame is handed off.
    record_finish_pending: bool,
    /// When the in-progress recording started — the origin for the
    /// throttle's logical timeline. `None` when not recording.
    record_started: Option<Instant>,
    /// Throttles the window's present-rate frame source down to the configured
    /// fps. `None` when not recording.
    record_throttle: Option<FrameThrottle>,
    /// When the previous frame was handed to the encoder, so each frame's
    /// on-screen duration is the real wall-clock gap since the last one.
    record_last_frame: Option<Instant>,
    /// Per-frame gap statistics for the in-progress recording — logged at
    /// stop to evidence real-time capture (vs the idle-window time-lapse).
    record_stats: FrameStats,
}

/// A tab drag in flight: the index the drag started on and the slot the
/// pointer is hovering now. The reorder is committed once, on release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TabDrag {
    from: usize,
    over: usize,
}

#[derive(Debug, Clone)]
enum Message {
    Window(window::Id, window::Event),
    ScanCompleted(Result<Vec<SessionRecord>, String>),
    /// The fs watcher saw the projects tree change (FR2).
    ProjectsChanged,
    /// A background plan/memory docs rediscovery finished (F-plans-memory).
    DocsDiscovered(Vec<DocEntry>),
    /// Unfold (or refold) a project's truncated session tail.
    ToggleExpanded(String),
    SearchChanged(String),
    SearchTitlesOnly(bool),
    /// Open a fresh shell in the given project directory (FR4a, `$` button).
    LaunchProject(String),
    /// Start a fresh Claude session in the given project directory (FR4a, 🤖
    /// button) — distinct from resuming an existing one.
    LaunchClaude(String),
    /// Resume a Claude session in its project directory (FR4).
    LaunchSession {
        cwd: String,
        resume: String,
    },
    /// New screen contents for a session.
    PtyOutput {
        session: SessionId,
        screen: Screen,
    },
    /// A session's activity was reclassified from the OSC stream (FR8).
    PtyStatus {
        session: SessionId,
        status: SessionStatus,
    },
    /// A session reported a new title over OSC; relabel its tab.
    PtyTitle {
        session: SessionId,
        title: String,
    },
    /// A session fired an OSC 9 notification; forward it to the OS.
    PtyNotify {
        session: SessionId,
        body: String,
    },
    /// A session's process exited.
    PtyExited(SessionId),
    /// A raw key press; routed to the focused terminal when it has focus.
    Key(keyboard::Event),
    /// IME-composed text (dead/accent keys, CJK) for the focused terminal.
    ImeCommit(String),
    /// Give keyboard focus to the terminal / the search box.
    FocusTerminal,
    FocusSearch,
    /// The mouse wheel turned over a terminal: the session under the pointer
    /// (not necessarily the focused one — splits), the pointer cell, and a line
    /// delta, so a mouse-mode app gets the wheel as input and a plain shell gets
    /// scrollback (FR4).
    TermScroll {
        session: SessionId,
        col: u16,
        row: u16,
        lines: i32,
    },
    /// Copy the given text (a terminal selection) to the clipboard (FR4).
    CopySelection(String),
    /// Clipboard contents read back for a paste into the focused terminal (FR4).
    Paste(Option<String>),
    /// Ask to close the tab at this index — arms the confirmation bar.
    RequestCloseTab(usize),
    /// Confirm the pending close, killing the tab's session(s) (FR5).
    CloseTab(usize),
    /// Dismiss the close confirmation without killing anything.
    CancelClose,
    /// A tab drag began on this index — the pointer pressed it (FR5).
    TabDragStart(usize),
    /// During a drag, the pointer entered the tab at this index.
    TabDragOver(usize),
    /// The drag's pointer was released: commit the reorder, else it was a
    /// plain click that activates the pressed tab.
    TabDragEnd,
    /// The drag left the tab strip without a drop — abandon it.
    TabDragCancel,
    /// Begin renaming a tab inline (double-click its chip), seeded with the
    /// title currently shown.
    StartTabRename {
        index: usize,
        current: String,
    },
    /// The inline tab-rename field's text changed.
    TabRenameInput(String),
    /// Commit the tab rename (Enter, or a blur onto another interaction).
    CommitTabRename,
    /// Abandon the tab rename (Escape), keeping the previous display title.
    CancelTabRename,
    /// Confirm quitting TermHerd, closing the window (and hard-killing every
    /// live session). Reached only after the quit modal is accepted.
    ConfirmCloseWindow,
    /// Dismiss the quit confirmation, keeping the app and its sessions running.
    CancelCloseWindow,
    /// Toggle a browsed session's star (F-session-metadata).
    ToggleStar(String),
    /// Toggle a project's star, by real path (F-favorites, repo-level).
    ToggleRepoStar(String),
    /// Toggle a browsed session's archived flag (F-session-metadata). Used
    /// directly only to un-archive (a harmless one-click restore); archiving
    /// goes through the confirmation flow below.
    ToggleArchive(String),
    /// Ask to archive a session — arms the confirmation bar.
    RequestArchive(String),
    /// Confirm the pending archive, hiding the session.
    ConfirmArchive,
    /// Dismiss the archive confirmation without archiving.
    CancelArchive,
    /// Show or hide archived sessions in the browser (F-session-metadata).
    ShowArchived(bool),
    /// Fold or unfold a project's session list in the sidebar, by path.
    ToggleCollapsed(String),
    /// Collapse or restore the whole session-browser sidebar.
    ToggleSidebar,
    /// Begin renaming a session inline, seeded with its current title.
    StartRename {
        session: String,
        current: String,
    },
    /// The inline rename field's text changed.
    RenameInput(String),
    /// Commit the inline rename (Enter or the ✓ button).
    CommitRename,
    /// Open a plan / memory doc in the main pane (F-plans-memory).
    OpenDoc {
        label: String,
        path: PathBuf,
    },
    /// A doc's contents finished loading, with the mtime captured at read.
    DocLoaded {
        label: String,
        path: PathBuf,
        content: String,
        mtime: Option<SystemTime>,
    },
    /// An edit/cursor action from the doc text editor.
    DocEdit(text_editor::Action),
    /// Save the open doc to disk (Save button or the save chord).
    SaveDoc,
    /// A save finished: the file's new mtime, or why it was refused.
    DocSaved(Result<SystemTime, crate::docs::SaveError>),
    /// Close the doc viewer, returning to the terminal.
    CloseDoc,
    /// Open a Ctrl/Cmd+clicked terminal link in the OS default handler.
    OpenUrl(String),
    /// The window screenshot for a capture finished; encode it to PNG at
    /// `png_path` (the companion of the already-written JSON dump). The encode
    /// runs off the UI thread, so this only spawns it.
    CaptureScreenshot {
        screenshot: window::Screenshot,
        png_path: PathBuf,
    },
    /// The capture PNG finished encoding off-thread: the path written, or
    /// the error to log.
    CaptureWritten(Result<PathBuf, String>),
    /// The window presented a frame while recording: the present clock
    /// from `window::frames()`. Throttled down to the configured fps, each kept
    /// tick asks `core` for the next frame / auto-stop decision. Driving capture
    /// off real presents (not a wall-clock timer) is what keeps an idle window's
    /// screenshots resolving in real time.
    RecordFrameTick(Instant),
    /// A recorded window screenshot is ready; hand it to the encoder
    /// thread.
    RecordFrame(window::Screenshot),
}

impl Message {
    /// Whether this message is a deliberate user interaction *elsewhere* in the
    /// UI that should cancel an in-progress inline rename. This is an explicit
    /// allowlist, not a blocklist: anything unlisted (PTY output, scans, window
    /// and key events, and the rename's own `StartRename`/`RenameInput`/
    /// `CommitRename`) leaves the edit untouched. Defaulting to "don't dismiss"
    /// is the safe side — a missed button is a minor gap, whereas a stray
    /// background message dismissing the edit would make renaming impossible.
    fn dismisses_rename(&self) -> bool {
        matches!(
            self,
            Self::SearchChanged(_)
                | Self::SearchTitlesOnly(_)
                | Self::LaunchProject(_)
                | Self::LaunchSession { .. }
                | Self::FocusTerminal
                | Self::FocusSearch
                | Self::TermScroll { .. }
                | Self::Paste(_)
                | Self::TabDragStart(_)
                | Self::TabDragEnd
                | Self::RequestCloseTab(_)
                | Self::CloseTab(_)
                | Self::ToggleStar(_)
                | Self::ToggleRepoStar(_)
                | Self::ToggleArchive(_)
                | Self::RequestArchive(_)
                | Self::ToggleCollapsed(_)
                | Self::ToggleExpanded(_)
                | Self::ToggleSidebar
                | Self::OpenDoc { .. }
                | Self::CloseDoc
        )
    }

    /// Whether this message is a deliberate interaction *elsewhere* that should
    /// commit an in-progress tab rename — the blur-commits convention (unlike a
    /// session rename, which blur cancels). `active` is the tab being renamed:
    /// the double-click that opened the edit emits `TabDragStart(active)` /
    /// `TabDragEnd` around it, and those must not commit; a press on a *different*
    /// tab, or focusing the terminal / search / launching, does.
    fn commits_tab_rename(&self, active: usize) -> bool {
        match self {
            // The double-click that opened the edit emits `TabDragStart(active)`
            // then `TabDragEnd` around it — its own drag noise must not commit. A
            // press on a *different* tab is a genuine blur.
            Self::TabDragStart(index) => *index != active,
            Self::TabDragEnd => false,
            // Every other deliberate interaction elsewhere that would dismiss a
            // session rename also commits a tab rename (the blur-commits
            // convention) — one shared allowlist, so the two can't drift.
            other => other.dismisses_rename(),
        }
    }
}

impl Shell {
    fn new(
        bounds: WindowConfig,
        scanner: Arc<dyn ProjectScanner>,
        watch_root: Option<PathBuf>,
        pty: Arc<dyn PtyHost>,
        pty_output: PtyOutput,
        startup: Startup,
    ) -> Self {
        let mut core = termherd_core::App::new();
        core.apply(termherd_core::Event::MetadataLoaded(startup.metadata));
        core.apply(termherd_core::Event::CollapsedLoaded(startup.collapsed));
        core.apply(termherd_core::Event::SessionLimitLoaded(
            startup.session_limit,
        ));
        core.apply(termherd_core::Event::FontSizeLoaded(startup.font_size));
        Self {
            core,
            bounds,
            scanner,
            watch_root,
            scan_error: None,
            pty,
            pty_output,
            screens: HashMap::new(),
            focus: Focus::Search,
            selection: None,
            theme: startup.theme.to_iced(),
            keymap: startup.keymap,
            renaming: None,
            tab_rename: None,
            // Populated by the first scan's `refresh_docs` — `discover` does
            // blocking fs I/O, which must stay off the UI thread.
            docs: Vec::new(),
            scan_in_flight: false,
            rescan_pending: false,
            open_doc: None,
            closing: None,
            close_confirm: startup.close,
            archiving: None,
            closing_window: None,
            link_modifier: false,
            tab_drag: None,
            exiting: false,
            record_config: startup.record,
            recorder: None,
            record_inflight: 0,
            record_finish_pending: false,
            record_started: None,
            record_throttle: None,
            record_last_frame: None,
            record_stats: FrameStats::default(),
        }
    }

    /// The GUI chrome theme (FR10); the terminal grid keeps its own colours.
    fn theme(&self) -> Theme {
        self.theme.clone()
    }

    /// Run one scan off the UI thread (FR2) and feed the result back. At most
    /// one scan runs at a time: changes seen while one is in flight coalesce
    /// into a single follow-up (`rescan_pending`), so a busy projects tree —
    /// a live Claude session appends to its JSONL continuously — can't stack
    /// overlapping scans.
    fn rescan(&mut self) -> Task<Message> {
        if self.scan_in_flight {
            self.rescan_pending = true;
            return Task::none();
        }
        self.scan_in_flight = true;
        let scanner = self.scanner.clone();
        Task::perform(
            async move { scanner.scan().map_err(|e| e.to_string()) },
            Message::ScanCompleted,
        )
    }

    /// A scan settled (success or failure): clear the in-flight flag and, if
    /// changes arrived while it ran, start the single follow-up scan they
    /// coalesced into.
    fn scan_settled(&mut self) -> Option<Task<Message>> {
        self.scan_in_flight = false;
        if self.rescan_pending {
            self.rescan_pending = false;
            Some(self.rescan())
        } else {
            None
        }
    }

    /// Rediscover the plan/memory docs off the UI thread (F-plans-memory).
    /// `discover` stats a `CLAUDE.md` per project path; on a dead path (an
    /// unplugged network mount, a removed directory) that stat can block for
    /// tens of seconds, so it must never run on the UI thread.
    fn refresh_docs(&self) -> Task<Message> {
        let paths: Vec<String> = self.core.projects.iter().map(|g| g.path.clone()).collect();
        Task::perform(
            async move { crate::docs::discover(&paths) },
            Message::DocsDiscovered,
        )
    }

    /// Carry out the effects `core` asked for, against the adapters. The PTY
    /// calls are quick (channel sends / a spawn); failures are logged, never
    /// fatal — a dead terminal must not take the app down (Q3).
    fn perform(&self, effects: Vec<Effect>) -> Task<Message> {
        for effect in effects {
            let outcome = match effect {
                Effect::Spawn(spec) => self.pty.spawn(spec),
                Effect::Write { session, bytes } => self.pty.write(session, &bytes),
                Effect::Resize {
                    session,
                    cols,
                    rows,
                } => self.pty.resize(session, cols, rows),
                Effect::Scroll { session, target } => self.pty.scroll(session, target),
                Effect::Kill(session) => self.pty.kill(session),
                // Metadata persistence is a file write, not a PTY call.
                Effect::SaveMetadata(metadata) => {
                    crate::metadata_store::save(&metadata);
                    Ok(())
                }
                // Fold state is a file write too.
                Effect::SaveCollapsed(collapsed) => {
                    crate::collapsed_store::save(&collapsed);
                    Ok(())
                }
                // Opening a link is an OS handoff, not a PTY call.
                Effect::OpenUrl(url) => open_url(&url),
                // A desktop notification is an OS handoff too.
                Effect::Notify { title, body } => notify(&title, &body),
                // Capture is performed by `Shell::capture`, not here: the JSON
                // dump and the PNG must share one timestamp, and the PNG needs
                // an async `window::screenshot` follow-up this fire-and-forget
                // loop can't return. Reaching this arm is unexpected — log it.
                Effect::Capture(_) => {
                    tracing::warn!("Effect::Capture routed through perform; ignored");
                    Ok(())
                }
                // Record effects are performed by `Shell::run_record_effects`,
                // not here: `CaptureFrame` needs an async `window::screenshot`
                // follow-up this loop can't return, and start/finish manage the
                // encoder thread. Reaching this arm is unexpected — log it.
                Effect::StartRecording
                | Effect::CaptureFrame
                | Effect::FinishRecording { .. }
                | Effect::CancelRecording => {
                    tracing::warn!("record effect routed through perform; ignored");
                    Ok(())
                }
            };
            if let Err(error) = outcome {
                tracing::warn!(%error, "pty effect failed");
            }
        }
        Task::none()
    }

    /// Launch a terminal: register it in `core`, perform the spawn, focus it,
    /// and size its PTY to the current pane (FR4).
    fn launch(&mut self, cwd: String, launch: Launch) -> Task<Message> {
        // The kind is shown as a suffix so a shell tab and a Claude tab for the
        // same repo stay distinct. Resuming a known session takes its real
        // name from the scanned digest — current Claude renders status
        // in-band and emits no OSC title, so the live override never fires;
        // without this every resumed tab in a repo would read alike. A fresh or
        // unscanned session keeps the kind label; an OSC title still wins later.
        let label = project_label(&cwd);
        let title = match &launch {
            Launch::Shell => format!("{label} $"),
            Launch::Claude {
                resume: Some(claude_id),
            } => self
                .core
                .record_for(claude_id)
                .map(|record| self.core.session_title(record))
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| format!("{label} 🤖")),
            Launch::Claude { resume: None } => format!("{label} 🤖"),
        };
        let effects = self
            .core
            .apply(termherd_core::Event::LaunchSession(LaunchSpec {
                cwd: Some(cwd),
                launch,
                title,
            }));
        let spawn = self.perform(effects);
        self.focus = Focus::Terminal;
        // Opening another session drops any pending confirmation: a
        // stray Enter in the terminal must not confirm a sidebar prompt that's
        // no longer in view.
        self.closing = None;
        self.archiving = None;
        Task::batch([spawn, self.resize_focused()])
    }

    /// The working directory of the focused session, if one is open and its cwd
    /// is known. The anchor for the "new in context" shortcuts.
    fn focused_cwd(&self) -> Option<String> {
        let id = self.core.workspace.focused_session()?;
        self.core.sessions.get(&id)?.cwd.clone()
    }

    /// Open a fresh shell in the focused session's directory, or in the
    /// home directory when nothing is open — so the shortcut still works from an
    /// empty workspace.
    fn new_shell_here(&mut self) -> Task<Message> {
        let cwd = self.focused_cwd().unwrap_or_else(home_dir);
        self.launch(cwd, Launch::Shell)
    }

    /// Open a fresh Claude session in the repo containing the focused session.
    /// Walks up to the repo root so a session running in a subdirectory
    /// still lands at the repo. Inert when nothing is open — there is no context
    /// to derive a repo from.
    fn new_claude_here(&mut self) -> Task<Message> {
        let Some(cwd) = self.focused_cwd() else {
            return Task::none();
        };
        let root = termherd_scan::repo_root(std::path::Path::new(&cwd))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(cwd);
        self.launch(root, Launch::Claude { resume: None })
    }

    /// Reopen the most recently closed tab, restoring its mode and
    /// directory. The reopen lives in `core`; here we just perform the spawn and
    /// focus the restored terminal, mirroring [`Self::launch`]. A no-op when the
    /// close stack is empty (`core` yields no effects).
    fn reopen_closed_tab(&mut self) -> Task<Message> {
        let effects = self.core.apply(termherd_core::Event::ReopenClosedTab);
        if effects.is_empty() {
            return Task::none();
        }
        let spawn = self.perform(effects);
        self.focus = Focus::Terminal;
        self.closing = None;
        self.archiving = None;
        Task::batch([spawn, self.resize_focused()])
    }

    /// The focused terminal's visible grid as text, for a capture. `None`
    /// when nothing is focused or its screen has not rendered yet — `core` then
    /// records a focus-less dump.
    fn focused_pty_text(&self) -> Option<String> {
        let id = self.core.workspace.focused_session()?;
        self.screens.get(&id).map(Screen::text)
    }

    /// Capture the current state for the AI dev loop: hand `core` the
    /// focused terminal's text, then write the JSON dump and take the PNG into
    /// `~/.termherd/captures/`. A no-op when no home directory is set.
    fn capture(&mut self) -> Task<Message> {
        let Some(dir) = crate::capture::captures_dir() else {
            tracing::warn!("no home directory; capture skipped");
            return Task::none();
        };
        let focused_pty_text = self.focused_pty_text();
        let effects = self
            .core
            .apply(termherd_core::Event::Capture { focused_pty_text });
        for effect in effects {
            if let Effect::Capture(dump) = effect {
                return self.perform_capture(&dir, dump);
            }
        }
        Task::none()
    }

    /// Write the rung-0 JSON dump into `dir` now and schedule the rung-1 PNG.
    /// Both share one timestamp so the pair is easy to find; the JSON is written
    /// synchronously (cheap), the PNG follows once iced returns the window
    /// screenshot. `dir` is a seam: production passes `~/.termherd/captures`,
    /// tests a throwaway. Any I/O failure is logged, never fatal — a missed
    /// capture must not take the app down.
    fn perform_capture(&self, dir: &std::path::Path, dump: CaptureDump) -> Task<Message> {
        if let Err(error) = std::fs::create_dir_all(dir) {
            tracing::warn!(%error, "could not create captures dir; capture skipped");
            return Task::none();
        }
        let stamp = crate::capture::stamp(SystemTime::now());
        match crate::capture::write_dump(dir, &stamp, &dump) {
            Ok(path) => tracing::info!(path = %path.display(), "capture dump written"),
            Err(error) => tracing::warn!(%error, "could not write capture dump"),
        }
        let png_path = crate::capture::png_path(dir, &stamp);
        // Screenshot the live window (rung 1), then encode + write the PNG.
        // `Task::<Option>::and_then` only fires for `Some`, so a window-less run
        // simply skips the PNG and the JSON dump still stands.
        window::latest()
            .and_then(window::screenshot)
            .map(move |screenshot| Message::CaptureScreenshot {
                screenshot,
                png_path: png_path.clone(),
            })
    }

    /// Start or stop the GIF screencast: hand `core` the frame cap from
    /// the record budget and perform whatever effects it returns. Ignored while a
    /// previous recording is still draining, so a back-to-back ⌘⇧R can't
    /// replace the recorder mid-finish.
    fn toggle_record(&mut self) -> Task<Message> {
        if self.record_toggle_blocked() {
            tracing::info!("record toggle ignored: previous screencast still finishing");
            return Task::none();
        }
        let max_frames = self.record_config.max_frames();
        let effects = self
            .core
            .apply(termherd_core::Event::ToggleRecord { max_frames });
        self.run_record_effects(effects)
    }

    /// Whether a ⌘⇧R press must be ignored because the previous recording is
    /// still draining its in-flight frames (problem 2). `core` has already
    /// returned to idle by the time a finish is pending, so without this guard a
    /// back-to-back start replaces `self.recorder` mid-finish — orphaning the
    /// first GIF (it finalises via `Drop`, but logs no `screencast written` and
    /// may be truncated).
    fn record_toggle_blocked(&self) -> bool {
        self.record_finish_pending
    }

    /// Perform the record effects `core` returned: open/feed/finish the
    /// encoder thread. `CaptureFrame` is the only one with an async follow-up —
    /// it screenshots the window, the result arriving as [`Message::RecordFrame`].
    fn run_record_effects(&mut self, effects: Vec<Effect>) -> Task<Message> {
        let mut task = Task::none();
        for effect in effects {
            match effect {
                Effect::StartRecording => self.start_recording(),
                Effect::CaptureFrame => {
                    self.record_inflight += 1;
                    task = window::latest()
                        .and_then(window::screenshot)
                        .map(Message::RecordFrame);
                }
                // `core` names the stop reason; logged the moment it happens (not
                // after the encoder drains) so start↔stop is unambiguous in the
                // trace.
                Effect::FinishRecording { capped } => {
                    let reason = if capped { "cap reached" } else { "manual" };
                    tracing::info!(reason, "screencast recording stopped");
                    self.request_finish_recording();
                }
                Effect::CancelRecording => {
                    tracing::info!("screencast recording cancelled (no frames captured)");
                    self.cancel_recording();
                }
                _ => {}
            }
        }
        task
    }

    /// Open the recorder thread for a new screencast, writing to
    /// `capture-<ts>.gif` in the capture dir. A missing home dir or an
    /// uncreatable dir aborts the start — logged, never fatal.
    fn start_recording(&mut self) {
        self.record_inflight = 0;
        self.record_finish_pending = false;
        // Fresh timing state for the throttle, the per-frame gap, and the stats.
        self.record_started = Some(Instant::now());
        self.record_throttle = Some(FrameThrottle::new(self.record_config.frame_interval()));
        self.record_last_frame = None;
        self.record_stats = FrameStats::default();
        let Some(dir) = crate::capture::captures_dir() else {
            tracing::warn!("no home directory; recording skipped");
            return;
        };
        if let Err(error) = std::fs::create_dir_all(&dir) {
            tracing::warn!(%error, "could not create captures dir; recording skipped");
            return;
        }
        let stamp = crate::capture::stamp(SystemTime::now());
        let path = dir.join(format!("capture-{stamp}.gif"));
        self.recorder = Some(Recorder::start(path, self.record_config));
        tracing::info!(
            fps = self.record_config.fps,
            cap_frames = self.record_config.max_frames(),
            "screencast recording started"
        );
    }

    /// A window present arrived while recording: throttle the present rate
    /// down to the configured fps and, on a kept tick, ask `core` for the next
    /// frame / auto-stop decision. Skipped ticks are dropped — they only served
    /// to keep the window presenting so the screenshot pipeline stays real-time.
    fn on_record_frame_tick(&mut self, now: Instant) -> Task<Message> {
        let (Some(started), Some(throttle)) = (self.record_started, self.record_throttle.as_mut())
        else {
            return Task::none();
        };
        if !throttle.should_capture(now.saturating_duration_since(started)) {
            return Task::none();
        }
        let effects = self.core.apply(termherd_core::Event::RecordTick);
        self.run_record_effects(effects)
    }

    /// Finish the screencast once every in-flight frame screenshot has been
    /// handed to the encoder, so a stop never drops the final frames. If
    /// none are in flight (a manual stop), finish straight away.
    fn request_finish_recording(&mut self) {
        if self.record_inflight == 0 {
            self.finish_recorder();
        } else {
            self.record_finish_pending = true;
        }
    }

    /// Flush and close the encoder thread, logging the frame-gap spread
    /// — the evidence that capture ran in real time (gaps ≈ the interval)
    /// rather than time-lapsed (gaps ballooning past it).
    fn finish_recorder(&mut self) {
        if let Some(recorder) = self.recorder.take() {
            recorder.finish();
        }
        if let Some(summary) = self.record_stats.summary() {
            tracing::info!(
                frames = summary.gaps,
                min_ms = summary.min.as_millis(),
                max_ms = summary.max.as_millis(),
                mean_ms = summary.mean.as_millis(),
                "screencast frame gaps"
            );
        }
        self.reset_record_state();
    }

    /// Abandon the screencast, deleting any partial file — the zero-frame
    /// stop.
    fn cancel_recording(&mut self) {
        if let Some(recorder) = self.recorder.take() {
            recorder.cancel();
        }
        self.reset_record_state();
    }

    /// Clear the per-recording runtime state once the encoder is done,
    /// returning the shell to idle.
    fn reset_record_state(&mut self) {
        self.record_inflight = 0;
        self.record_finish_pending = false;
        self.record_started = None;
        self.record_throttle = None;
        self.record_last_frame = None;
        self.record_stats = FrameStats::default();
    }

    /// Hand one recorded window screenshot to the encoder thread, then
    /// finish if this was the last frame a stop was waiting on. The gap since the
    /// previous handed frame becomes this frame's on-screen duration, and
    /// is folded into the stats — the gaps are the present-gating measurement.
    fn on_record_frame(&mut self, screenshot: window::Screenshot) -> Task<Message> {
        let now = Instant::now();
        let previous = self.record_last_frame;
        // The first frame has no predecessor, so it shows for the nominal
        // interval; later frames show for the real gap since the last one.
        let gap = previous.map_or_else(
            || self.record_config.frame_interval(),
            |last| now.saturating_duration_since(last),
        );
        if let Some(recorder) = self.recorder.as_ref() {
            let (width, height) = (screenshot.size.width, screenshot.size.height);
            recorder.frame(screenshot.rgba.to_vec(), width, height, gap);
            // Only *measured* gaps count toward the benchmark — the first frame's
            // nominal interval isn't a real arrival gap and would skew min/mean.
            if previous.is_some() {
                self.record_stats.record_gap(gap);
            }
            self.record_last_frame = Some(now);
        }
        self.record_inflight = self.record_inflight.saturating_sub(1);
        if self.record_finish_pending && self.record_inflight == 0 {
            self.finish_recorder();
        }
        Task::none()
    }

    /// Move the focused terminal's viewport: the mouse wheel sends a
    /// relative delta, the scroll-top/bottom shortcuts an absolute jump. Shared
    /// so both paths go through the one `Event::ScrollViewport`.
    fn scroll_focused(&mut self, target: ScrollTarget) -> Task<Message> {
        let Some(session) = self.core.workspace.focused_session() else {
            return Task::none();
        };
        self.scroll_session(session, target)
    }

    /// Move a specific session's viewport. The wheel targets the pane under the
    /// pointer, which need not be the focused one in a split layout.
    fn scroll_session(&mut self, session: SessionId, target: ScrollTarget) -> Task<Message> {
        let effects = self
            .core
            .apply(termherd_core::Event::ScrollViewport { session, target });
        self.perform(effects)
    }

    /// Tell the focused session's PTY to match the current pane geometry.
    fn resize_focused(&mut self) -> Task<Message> {
        let Some(session) = self.core.workspace.focused_session() else {
            return Task::none();
        };
        let (cols, rows) = self.grid_size();
        let effects = self.core.apply(termherd_core::Event::TerminalResized {
            session,
            cols,
            rows,
        });
        self.perform(effects)
    }

    /// Collapse or restore the sidebar, then resize the focused terminal
    /// so the grid re-derives its column count for the new width — without this
    /// the cells just stretch to fill the reclaimed space. Shared by the
    /// button (`Message::ToggleSidebar`) and the keymap (`Action::ToggleSidebar`).
    fn toggle_sidebar(&mut self) -> Task<Message> {
        let _ = self.core.apply(termherd_core::Event::ToggleSidebar);
        self.resize_focused()
    }

    /// Zoom the terminal font, then resize the focused terminal so the
    /// grid re-derives its cols/rows for the new cell box — the same pattern
    /// as [`Self::toggle_sidebar`].
    fn zoom(&mut self, zoom: termherd_core::Zoom) -> Task<Message> {
        let _ = self.core.apply(termherd_core::Event::Zoom(zoom));
        self.resize_focused()
    }

    /// The terminal grid size (cols, rows) that fits the current window. The
    /// sidebar's width is only reserved while it's visible; collapsing it
    /// hands that space to the grid as extra columns instead of stretching the
    /// existing cells.
    fn grid_size(&self) -> (u16, u16) {
        let sidebar = if self.core.sidebar_hidden {
            HANDLE_W
        } else {
            SIDEBAR_W
        };
        let (cell_w, cell_h) = cell_size(self.core.font_size());
        let avail_w = (self.bounds.width - sidebar - H_CHROME).max(cell_w);
        let avail_h = (self.bounds.height - V_CHROME).max(cell_h);
        let cols = (avail_w / cell_w).floor().clamp(20.0, 500.0) as u16;
        let rows = (avail_h / cell_h).floor().clamp(5.0, 200.0) as u16;
        (cols, rows)
    }

    // The iced `update` is a flat `match` over every `Message` variant — the
    // app's central event dispatcher. Length here is breadth (one arm per
    // message), not nested complexity; splitting it would scatter the dispatch.
    // Tracked as a refactor candidate (shell.rs is the god-object signal A).
    #[allow(clippy::too_many_lines)]
    fn update(&mut self, message: Message) -> Task<Message> {
        // Clicking (or typing) anywhere else in TermHerd while an inline rename
        // is open discards it — the blur-cancels-edit convention. Only genuine
        // user interactions dismiss it; background traffic (PTY output,
        // rescans, window events) and the rename's own messages must not, or a
        // chatty terminal would cancel the edit before it could be typed.
        if self.renaming.is_some() && message.dismisses_rename() {
            self.renaming = None;
        }
        // A tab rename blurs the other way: a genuine interaction elsewhere
        // *commits* the pending name (the double-click's own drag noise on the
        // same tab is excluded by `commits_tab_rename`), then the message itself
        // still dispatches below. The anchored tab's current index feeds that
        // drag-noise discrimination; `usize::MAX` (the tab is gone) never
        // matches a real `TabDragStart`, so any interaction just commits.
        if let Some(anchor) = self.tab_rename.as_ref().map(|(a, _)| *a) {
            let active = self
                .core
                .workspace
                .tab_of_session(anchor)
                .unwrap_or(usize::MAX);
            if message.commits_tab_rename(active) {
                self.commit_tab_rename();
            }
        }
        match message {
            Message::Window(id, event) => self.on_window_event(id, event),
            Message::ScanCompleted(Ok(records)) => {
                tracing::info!(sessions = records.len(), "scan completed");
                self.scan_error = None;
                let effects = self
                    .core
                    .apply(termherd_core::Event::ScanCompleted(records));
                debug_assert!(effects.is_empty());
                // If changes arrived mid-scan, the coalesced follow-up scan
                // will refresh the docs itself; otherwise refresh them now
                // that the project paths are known (a project's CLAUDE.md
                // sits in its real directory).
                match self.scan_settled() {
                    Some(next_scan) => next_scan,
                    None => self.refresh_docs(),
                }
            }
            Message::ScanCompleted(Err(error)) => {
                tracing::warn!(%error, "scan failed");
                self.scan_error = Some(error);
                // Even on failure, discover the global docs (memory, plans) so
                // the docs pane isn't empty when the very first scan fails.
                match self.scan_settled() {
                    Some(next_scan) => next_scan,
                    None => self.refresh_docs(),
                }
            }
            Message::DocsDiscovered(docs) => {
                self.docs = docs;
                Task::none()
            }
            Message::ProjectsChanged => {
                tracing::debug!("projects tree changed; rescanning");
                self.rescan()
            }
            Message::SearchChanged(query) => {
                let _ = self.core.apply(termherd_core::Event::SearchChanged(query));
                Task::none()
            }
            Message::SearchTitlesOnly(titles_only) => {
                let _ = self
                    .core
                    .apply(termherd_core::Event::SearchTitlesOnlyToggled(titles_only));
                Task::none()
            }
            Message::LaunchProject(cwd) => self.launch(cwd, Launch::Shell),
            Message::LaunchClaude(cwd) => self.launch(cwd, Launch::Claude { resume: None }),
            Message::LaunchSession { cwd, resume } => {
                // Re-clicking a session already open in TermHerd re-focuses its
                // tab instead of spawning a second terminal for the same Claude
                // session (FR4).
                if let Some(session) = self.core.open_session_for(&resume)
                    && let Some(index) = self.core.workspace.tab_of(session)
                {
                    return self.activate_tab(index);
                }
                self.launch(
                    cwd,
                    Launch::Claude {
                        resume: Some(resume),
                    },
                )
            }
            Message::PtyOutput { session, screen } => {
                self.screens.insert(session, screen);
                Task::none()
            }
            Message::PtyStatus { session, status } => {
                let _ = self
                    .core
                    .apply(termherd_core::Event::StatusChanged { session, status });
                Task::none()
            }
            Message::PtyTitle { session, title } => {
                let _ = self
                    .core
                    .apply(termherd_core::Event::SessionTitleChanged { session, title });
                Task::none()
            }
            Message::PtyNotify { session, body } => {
                // Unlike status/title, this yields an `Effect::Notify` that the
                // shell must perform — hand it to the OS notification centre.
                let effects = self
                    .core
                    .apply(termherd_core::Event::SessionNotified { session, body });
                self.perform(effects)
            }
            Message::PtyExited(session) => {
                let _ = self.core.apply(termherd_core::Event::PtyExited(session));
                Task::none()
            }
            Message::Key(event) => {
                // Keep the link-open modifier state current regardless of focus,
                // so a Ctrl/Cmd+hover highlights links even before the first key
                // reaches the terminal.
                let modifiers = event_modifiers(&event);
                self.link_modifier = modifiers.control() || modifiers.logo();
                self.on_key(event)
            }
            Message::ImeCommit(text) => self.on_ime_commit(text),
            Message::FocusTerminal => {
                self.focus = Focus::Terminal;
                Task::none()
            }
            Message::FocusSearch => {
                self.focus = Focus::Search;
                operate(focusable::focus(search_id()))
            }
            Message::TermScroll {
                session,
                col,
                row,
                lines,
            } => self.scroll_session(session, ScrollTarget::Wheel { col, row, lines }),
            Message::CopySelection(text) => {
                if text.is_empty() {
                    Task::none()
                } else {
                    self.selection = Some(text.clone());
                    iced::clipboard::write(text)
                }
            }
            Message::Paste(content) => {
                let Some(text) = content.filter(|t| !t.is_empty()) else {
                    return Task::none();
                };
                let Some(session) = self.core.workspace.focused_session() else {
                    return Task::none();
                };
                let bracketed = self
                    .screens
                    .get(&session)
                    .is_some_and(|screen| screen.bracketed_paste);
                let effects = self.core.apply(termherd_core::Event::TerminalInput {
                    session,
                    bytes: termherd_pty::paste_bytes(&text, bracketed),
                });
                self.perform(effects)
            }
            Message::RequestCloseTab(index) => self.request_close(index),
            Message::CloseTab(index) => self.close_tab(index),
            Message::CancelClose => {
                self.closing = None;
                Task::none()
            }
            Message::TabDragStart(index) => {
                if index < self.core.workspace.tabs.len() {
                    self.tab_drag = Some(TabDrag {
                        from: index,
                        over: index,
                    });
                }
                Task::none()
            }
            Message::TabDragOver(index) => {
                if let Some(drag) = self.tab_drag.as_mut()
                    && index < self.core.workspace.tabs.len()
                {
                    drag.over = index;
                }
                Task::none()
            }
            Message::TabDragEnd => match self.tab_drag.take() {
                // A real drag (the pointer crossed onto another tab): reorder.
                Some(TabDrag { from, over }) if from != over => {
                    let _ = self
                        .core
                        .apply(termherd_core::Event::MoveTab { from, to: over });
                    Task::none()
                }
                // No movement — the press/release was a plain click: activate.
                Some(TabDrag { from, .. }) => self.activate_tab(from),
                None => Task::none(),
            },
            Message::TabDragCancel => {
                self.tab_drag = None;
                Task::none()
            }
            Message::StartTabRename { index, current } => {
                // Anchor on the tab's first session so the edit survives a
                // reorder; every tab hosts at least one, so this is `Some` for a
                // valid index.
                if let Some(anchor) = self
                    .core
                    .workspace
                    .tabs
                    .get(index)
                    .and_then(|tab| tab.sessions().first().copied())
                {
                    self.tab_rename = Some((anchor, current));
                    return operate(focusable::focus(tab_rename_id()));
                }
                Task::none()
            }
            Message::TabRenameInput(value) => {
                if let Some((_, buffer)) = &mut self.tab_rename {
                    *buffer = value;
                }
                Task::none()
            }
            Message::CommitTabRename => {
                self.commit_tab_rename();
                Task::none()
            }
            Message::CancelTabRename => {
                self.tab_rename = None;
                Task::none()
            }
            Message::ConfirmCloseWindow => match self.closing_window.take() {
                Some(_) => {
                    self.exiting = true;
                    iced::exit()
                }
                None => Task::none(),
            },
            Message::CancelCloseWindow => {
                self.closing_window = None;
                Task::none()
            }
            Message::ToggleStar(session) => {
                let effects = self.core.apply(termherd_core::Event::ToggleStar(session));
                self.perform(effects)
            }
            Message::ToggleRepoStar(path) => {
                let effects = self.core.apply(termherd_core::Event::ToggleRepoStar(path));
                self.perform(effects)
            }
            Message::ToggleArchive(session) => {
                let effects = self
                    .core
                    .apply(termherd_core::Event::ToggleArchive(session));
                self.perform(effects)
            }
            Message::RequestArchive(session) => {
                self.archiving = Some(session);
                Task::none()
            }
            Message::ConfirmArchive => match self.archiving.take() {
                // Only archive a session still on the scanned list: a rescan
                // could have dropped it while the prompt was up, and toggling a
                // vanished id would persist phantom metadata for it.
                Some(session) if self.is_browsable(&session) => {
                    let effects = self
                        .core
                        .apply(termherd_core::Event::ToggleArchive(session));
                    self.perform(effects)
                }
                _ => Task::none(),
            },
            Message::CancelArchive => {
                self.archiving = None;
                Task::none()
            }
            Message::ShowArchived(show) => {
                let _ = self
                    .core
                    .apply(termherd_core::Event::ShowArchivedToggled(show));
                Task::none()
            }
            Message::ToggleCollapsed(path) => {
                let effects = self.core.apply(termherd_core::Event::ToggleCollapsed(path));
                self.perform(effects)
            }
            Message::ToggleExpanded(path) => {
                let _ = self.core.apply(termherd_core::Event::ToggleExpanded(path));
                Task::none()
            }
            Message::ToggleSidebar => self.toggle_sidebar(),
            Message::StartRename { session, current } => {
                self.renaming = Some((session, current));
                operate(focusable::focus(rename_id()))
            }
            Message::RenameInput(value) => {
                if let Some((_, buffer)) = &mut self.renaming {
                    *buffer = value;
                }
                Task::none()
            }
            Message::CommitRename => match self.renaming.take() {
                Some((session, title)) => {
                    let effects = self
                        .core
                        .apply(termherd_core::Event::RenameSession { session, title });
                    self.perform(effects)
                }
                None => Task::none(),
            },
            Message::OpenDoc { label, path } => {
                let read_path = path.clone();
                Task::perform(
                    async move {
                        let content = crate::docs::read(&read_path)
                            .unwrap_or_else(crate::strings::doc_read_failed);
                        let mtime = crate::docs::mtime(&read_path).ok();
                        (content, mtime)
                    },
                    move |(content, mtime)| Message::DocLoaded {
                        label: label.clone(),
                        path: path.clone(),
                        content,
                        mtime,
                    },
                )
            }
            Message::DocLoaded {
                label,
                path,
                content,
                mtime,
            } => {
                let writable = crate::docs::is_writable(&path);
                self.open_doc = Some(OpenDoc {
                    label,
                    path,
                    content: text_editor::Content::with_text(&content),
                    loaded_mtime: mtime,
                    writable,
                    dirty: false,
                    feedback: None,
                });
                Task::none()
            }
            Message::DocEdit(action) => {
                if let Some(doc) = &mut self.open_doc {
                    let edits = action.is_edit();
                    doc.content.perform(action);
                    if edits {
                        doc.dirty = true;
                        doc.feedback = None;
                    }
                }
                Task::none()
            }
            Message::SaveDoc => self.save_open_doc(),
            Message::DocSaved(result) => {
                if let Some(doc) = &mut self.open_doc {
                    match result {
                        Ok(new_mtime) => {
                            doc.loaded_mtime = Some(new_mtime);
                            doc.dirty = false;
                            doc.feedback = Some(DocFeedback::Saved);
                        }
                        Err(error) => {
                            doc.feedback = Some(DocFeedback::Error(error.to_string()));
                        }
                    }
                }
                Task::none()
            }
            Message::CloseDoc => {
                self.open_doc = None;
                Task::none()
            }
            Message::OpenUrl(url) => {
                let effects = self.core.apply(termherd_core::Event::OpenUrl(url));
                self.perform(effects)
            }
            Message::CaptureScreenshot {
                screenshot,
                png_path,
            } => {
                // Encoding a multi-megapixel RGBA buffer to PNG is tens to
                // hundreds of ms; run it off the runtime thread so ⌘⇧S doesn't
                // freeze the UI (the screenshot itself is refcounted `Bytes`,
                // cheap to hand off).
                Task::perform(
                    async move {
                        crate::capture::write_png(&png_path, &screenshot)
                            .map(|()| png_path)
                            .map_err(|error| error.to_string())
                    },
                    Message::CaptureWritten,
                )
            }
            Message::CaptureWritten(result) => {
                match result {
                    Ok(path) => {
                        tracing::info!(path = %path.display(), "capture screenshot written");
                    }
                    Err(error) => tracing::warn!(%error, "could not write capture screenshot"),
                }
                Task::none()
            }
            Message::RecordFrameTick(now) => self.on_record_frame_tick(now),
            Message::RecordFrame(screenshot) => self.on_record_frame(screenshot),
        }
    }

    /// Save the open doc off-thread, if there is one with unsaved edits in the
    /// writable scope. A no-op otherwise, so the save chord/button is harmless
    /// when nothing needs writing.
    fn save_open_doc(&self) -> Task<Message> {
        let Some(doc) = &self.open_doc else {
            return Task::none();
        };
        if !doc.writable || !doc.dirty {
            return Task::none();
        }
        let path = doc.path.clone();
        let contents = doc.content.text();
        let open_mtime = doc.loaded_mtime;
        Task::perform(
            async move { crate::docs::save(&path, &contents, open_mtime) },
            Message::DocSaved,
        )
    }

    /// Whether a session id is still on the scanned project list — used to
    /// guard the archive confirmation against a session a rescan removed while
    /// the prompt was up.
    fn is_browsable(&self, session: &str) -> bool {
        self.core
            .projects
            .iter()
            .any(|group| group.sessions.iter().any(|s| s.session_id == session))
    }

    /// Handle a request to close the tab at `index`. The configured `close.tab`
    /// policy decides: arm the confirmation bar or close straight away.
    /// `confirmWhenActive` (the default) keys off the core
    /// `tab_has_running_process` predicate — an idle tab has nothing to lose and
    /// closes silently, a running one confirms; `alwaysConfirm` / `noConfirmation`
    /// override that. No-op for an out-of-range index, so a stale request can
    /// never close the wrong tab.
    fn request_close(&mut self, index: usize) -> Task<Message> {
        // A pending confirmation owns the interaction (like the keyboard in
        // `on_key`): while one is up, ignore a close request for another tab so
        // it can't silently close that tab and drop the unanswered prompt.
        if self.closing.is_some() {
            return Task::none();
        }
        if index >= self.core.workspace.tabs.len() {
            return Task::none();
        }
        if self
            .close_confirm
            .tab
            .confirms(self.core.tab_has_running_process(index))
        {
            self.closing = Some(index);
            Task::none()
        } else {
            self.close_tab(index)
        }
    }

    /// Close the tab at `index`, killing its session(s) (FR5). Reached only
    /// after the confirmation is accepted: the close button and the
    /// `CloseFocused` keymap action both arm `closing` first.
    fn close_tab(&mut self, index: usize) -> Task<Message> {
        self.closing = None;
        // Capture the sessions about to die so their cached screens don't
        // outlive them in the shell.
        let dying = self
            .core
            .workspace
            .tabs
            .get(index)
            .map(|tab| tab.sessions())
            .unwrap_or_default();
        let effects = self.core.apply(termherd_core::Event::CloseTab(index));
        for id in dying {
            self.screens.remove(&id);
        }
        let kill = self.perform(effects);
        Task::batch([kill, self.resize_focused()])
    }

    /// Copy the last terminal selection to the clipboard, if any (FR4).
    fn copy_selection(&self) -> Task<Message> {
        match &self.selection {
            Some(sel) if !sel.is_empty() => iced::clipboard::write(sel.clone()),
            _ => Task::none(),
        }
    }

    /// Switch to the tab at `index` and return focus to the terminal. Switching
    /// drops any pending confirmation. An out-of-range index is a
    /// silent no-op in `core`, so a number key with no matching tab does
    /// nothing.
    fn activate_tab(&mut self, index: usize) -> Task<Message> {
        let _ = self.core.apply(termherd_core::Event::ActivateTab(index));
        self.focus = Focus::Terminal;
        self.closing = None;
        self.archiving = None;
        self.resize_focused()
    }

    /// Apply the pending tab rename to the core and clear the edit. The core's
    /// [`rename_tab`] owns the naming rules — a blank name (or one equal to the
    /// derived title) reverts to the derived title rather than freezing it, so
    /// an accidental double-click + Enter leaves the tab dynamic. The index is
    /// resolved *fresh* from the anchor session, since it may have shifted (or
    /// the tab vanished) since the edit began. No-op when nothing is pending or
    /// the anchored tab is gone.
    ///
    /// [`rename_tab`]: termherd_core::workspace::Workspace::rename_tab
    fn commit_tab_rename(&mut self) {
        let Some((anchor, title)) = self.tab_rename.take() else {
            return;
        };
        let Some(index) = self.core.workspace.tab_of_session(anchor) else {
            return;
        };
        let _ = self
            .core
            .apply(termherd_core::Event::RenameTab { index, title });
    }

    /// Switch the active tab by `delta`, wrapping around (FR9 `NextTab` /
    /// `PrevTab`). No-op when nothing is open.
    fn cycle_tab(&mut self, delta: i32) -> Task<Message> {
        let count = self.core.workspace.tabs.len();
        if count == 0 {
            return Task::none();
        }
        let next = (self.core.workspace.active as i32 + delta).rem_euclid(count as i32) as usize;
        self.activate_tab(next)
    }

    /// Run a keymap [`Action`] (FR9). Clipboard actions become iced tasks; tab
    /// actions drive `core`. Actions without a surface yet are no-ops.
    fn run_action(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::Copy => self.copy_selection(),
            Action::Paste => iced::clipboard::read().map(Message::Paste),
            Action::NextTab => self.cycle_tab(1),
            Action::PrevTab => self.cycle_tab(-1),
            Action::CloseFocused => self.request_close(self.core.workspace.active),
            Action::FocusSearch => {
                self.focus = Focus::Search;
                operate(focusable::focus(search_id()))
            }
            Action::ToggleSidebar => self.toggle_sidebar(),
            Action::ScrollTop => self.scroll_focused(ScrollTarget::Top),
            Action::ScrollBottom => self.scroll_focused(ScrollTarget::Bottom),
            // New shell / Claude session in the focused context, and
            // reopen the last closed tab.
            Action::NewShellHere => self.new_shell_here(),
            Action::NewClaudeSessionHere => self.new_claude_here(),
            Action::ReopenClosedTab => self.reopen_closed_tab(),
            // Capture the current state for the AI dev loop.
            Action::Capture => self.capture(),
            // Start / stop the GIF screencast.
            Action::ToggleRecord => self.toggle_record(),
            // Zoom re-derives the grid geometry, so the focused terminal is
            // resized like on a window resize; other tabs catch up on
            // focus, the existing convention.
            Action::ZoomIn => self.zoom(termherd_core::Zoom::In),
            Action::ZoomOut => self.zoom(termherd_core::Zoom::Out),
            Action::ZoomReset => self.zoom(termherd_core::Zoom::Reset),
            // Number-row jump straight to a tab. An index past the
            // open tabs is absorbed by `core` as a no-op.
            Action::ActivateTab(index) => self.activate_tab(index),
            Action::OpenNewSession
            | Action::SplitHorizontal
            | Action::SplitVertical
            | Action::FocusNext
            | Action::FocusPrev => Task::none(),
        }
    }

    /// Route a key press to the focused terminal's PTY (FR4). Ignored unless a
    /// terminal holds focus, so the search box keeps its own typing.
    fn on_key(&mut self, event: keyboard::Event) -> Task<Message> {
        // While renaming a tab inline, the text field owns the keyboard —
        // except Escape, which abandons the edit (Enter commits via the field's
        // own submit; a blur commits through `commits_tab_rename`).
        if self.tab_rename.is_some() {
            if let keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            } = &event
            {
                return self.update(Message::CancelTabRename);
            }
            return Task::none();
        }
        // While renaming inline, let the text field own the keyboard.
        if self.renaming.is_some() {
            return Task::none();
        }
        // The quit modal owns the keyboard while it is up: Enter quits, Escape
        // cancels, every other key is swallowed. Checked first so it wins over
        // the tab/archive prompts beneath it.
        if self.quit_pending() {
            if let keyboard::Event::KeyPressed { key, .. } = &event {
                match key {
                    keyboard::Key::Named(keyboard::key::Named::Enter) => {
                        return self.update(Message::ConfirmCloseWindow);
                    }
                    keyboard::Key::Named(keyboard::key::Named::Escape) => {
                        self.closing_window = None;
                    }
                    _ => {}
                }
            }
            return Task::none();
        }
        // A pending close confirmation captures the keyboard: Enter
        // confirms, Escape cancels, and every other key is swallowed so a
        // keystroke can't slip past to the terminal while the prompt is up.
        if let Some(index) = self.closing {
            if let keyboard::Event::KeyPressed { key, .. } = &event {
                match key {
                    keyboard::Key::Named(keyboard::key::Named::Enter) => {
                        return self.close_tab(index);
                    }
                    keyboard::Key::Named(keyboard::key::Named::Escape) => {
                        self.closing = None;
                    }
                    _ => {}
                }
            }
            return Task::none();
        }
        // A pending archive confirmation likewise owns the keyboard:
        // Enter archives, Escape cancels, other keys are swallowed.
        if self.archiving.is_some() {
            if let keyboard::Event::KeyPressed { key, .. } = &event {
                match key {
                    keyboard::Key::Named(keyboard::key::Named::Enter) => {
                        return self.update(Message::ConfirmArchive);
                    }
                    keyboard::Key::Named(keyboard::key::Named::Escape) => {
                        self.archiving = None;
                    }
                    _ => {}
                }
            }
            return Task::none();
        }
        // An open doc owns the keyboard: the text editor handles keys itself, so
        // swallow them here (never leak to the terminal underneath), but honour
        // the save chord (Cmd/Ctrl+S).
        if self.open_doc.is_some() {
            if let keyboard::Event::KeyPressed { key, modifiers, .. } = &event
                && modifiers.command()
                && matches!(key, keyboard::Key::Character(c) if c.as_str() == "s")
            {
                return self.save_open_doc();
            }
            return Task::none();
        }
        let keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            text,
            location,
            ..
        } = event
        else {
            return Task::none();
        };
        // A configured shortcut wins over raw terminal input: build the chord
        // and run its action if the keymap binds one (FR9). Resolved before the
        // terminal-focus guard so command chords are global — `mod+T` opens the
        // first shell even from an empty workspace with the search box focused.
        // Run-action handlers that need a session guard for one
        // themselves. Unbound keys fall through to the terminal, so plain Ctrl+C
        // stays the interrupt signal.
        if let Some(chord) = chord_of(&key, &physical_key, modifiers)
            && let Some(action) = self.keymap.lookup(&chord)
        {
            return self.run_action(action);
        }
        if self.focus != Focus::Terminal {
            return Task::none();
        }
        let Some(session) = self.core.workspace.focused_session() else {
            return Task::none();
        };
        // A numpad key with NumLock on reports its un-locked name (`End`, arrows,
        // …) but carries the digit/operator in `text`; type that instead of the
        // navigation sequence its name would otherwise produce. Other keys map
        // by name as usual.
        let term_key = numpad_char(location, text.as_deref())
            .map(TermKey::Char)
            .or_else(|| to_term_key(&key));
        let Some(term_key) = term_key else {
            return Task::none();
        };
        let Some(bytes) = termherd_pty::key_bytes(term_key, key_mods(modifiers), text.as_deref())
        else {
            return Task::none();
        };
        let effects = self
            .core
            .apply(termherd_core::Event::TerminalInput { session, bytes });
        self.perform(effects)
    }

    /// Whether raw keyboard / IME input should reach the focused terminal: it
    /// holds focus and no overlay (inline rename, close confirmation) is up.
    /// Focus stays `Terminal` while those overlays are open, so they have to be
    /// excluded explicitly — this is the predicate [`Shell::on_key`] enforces
    /// step by step, shared so the IME path can't drift from it.
    fn accepts_terminal_input(&self) -> bool {
        self.focus == Focus::Terminal
            && self.renaming.is_none()
            && self.tab_rename.is_none()
            && self.closing.is_none()
            && self.archiving.is_none()
            && self.open_doc.is_none()
            && !self.quit_pending()
    }

    /// Route IME-composed text (dead/accent keys, CJK) to the focused terminal
    /// as typed bytes. A commit only fires while the terminal accepts
    /// input (see [`Shell::accepts_terminal_input`]), but guard anyway so a
    /// composing overlay (rename / close confirmation) keeps its own typing.
    fn on_ime_commit(&mut self, text: String) -> Task<Message> {
        if !self.accepts_terminal_input() || text.is_empty() {
            return Task::none();
        }
        let Some(session) = self.core.workspace.focused_session() else {
            return Task::none();
        };
        let effects = self.core.apply(termherd_core::Event::TerminalInput {
            session,
            bytes: text.into_bytes(),
        });
        self.perform(effects)
    }

    fn on_window_event(&mut self, id: window::Id, event: window::Event) -> Task<Message> {
        match event {
            window::Event::Opened { .. } => {
                // Reroute the macOS menu Quit item (and ⌘Q) through the iced
                // runtime. Done here, not in the boot closure: iced constructs
                // the app state *before* `run_app`, so the boot closure runs
                // ahead of winit's `applicationDidFinishLaunching` (where the
                // default menu is installed). By the time the window is `Opened`
                // the event loop is running and the menu exists, and we are on
                // the main thread. Fires once (single window); no-op on other
                // platforms.
                #[cfg(target_os = "macos")]
                match objc2_foundation::MainThreadMarker::new() {
                    Some(mtm) => crate::macos::route_quit_through_close(mtm),
                    // We expect to be on the main thread here; if not, skipping
                    // would silently leave Cmd+Q on AppKit's hard-kill
                    // `terminate:` with no trace explaining why. Log it.
                    None => tracing::warn!(
                        "window Opened off the main thread; Cmd+Q stays on AppKit terminate:"
                    ),
                }
                Task::none()
            }
            window::Event::Moved(position) => {
                self.bounds.x = Some(position.x);
                self.bounds.y = Some(position.y);
                Task::none()
            }
            window::Event::Resized(size) => {
                self.bounds.width = size.width;
                self.bounds.height = size.height;
                self.resize_focused()
            }
            window::Event::CloseRequested => {
                self.bounds.save();
                self.request_quit(id)
            }
            window::Event::Focused => {
                let _ = self
                    .core
                    .apply(termherd_core::Event::WindowFocusChanged(true));
                Task::none()
            }
            window::Event::Unfocused => {
                let _ = self
                    .core
                    .apply(termherd_core::Event::WindowFocusChanged(false));
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The single convergence point for every way the user can quit TermHerd.
    /// All three macOS triggers — the window-close button, the menu Quit item,
    /// and Cmd+Q — arrive here as a `CloseRequested` window event: the menu
    /// Quit action is repointed from AppKit's `terminate:` to `performClose:`
    /// at startup (`crate::macos`), so it routes through winit's
    /// `windowShouldClose:` like the close button instead of terminating the
    /// process out from under us. Keeping one seam is the structural fix — a
    /// second, unguarded quit path is exactly the defect this prevents.
    ///
    /// A quit hard-kills every live session's foreground process. Whether it
    /// confirms first is the configured app policy: `confirmWhenActive` (the
    /// default) confirms only while some session is still running work — the
    /// core `any_running_process` predicate, the app-wide sibling of the one the
    /// tab close uses — so an all-idle app quits silently; `alwaysConfirm` /
    /// `noConfirmation` override that. `iced::exit` (not `window::close`) is what
    /// actually ends the process: on macOS winit cancels the OS terminate and
    /// `exit_on_close_request(false)` keeps the runtime alive, so a mere window
    /// close would survive.
    fn request_quit(&mut self, id: window::Id) -> Task<Message> {
        if self
            .close_confirm
            .app
            .confirms(self.core.any_running_process())
        {
            self.closing_window = Some(id);
            Task::none()
        } else {
            tracing::info!("quit needs no confirmation; exiting");
            self.exiting = true;
            iced::exit()
        }
    }

    /// Whether a quit is awaiting confirmation (the modal is up).
    fn quit_pending(&self) -> bool {
        self.closing_window.is_some()
    }

    /// Count of sessions whose PTY is still running — the ones a quit would
    /// hard-kill. Exited sessions linger in the map but cost nothing to drop.
    fn live_session_count(&self) -> usize {
        self.core
            .sessions
            .values()
            .filter(|s| s.status != SessionStatus::Exited)
            .count()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            window::events().map(|(id, event)| Message::Window(id, event)),
            keyboard::listen().map(Message::Key),
        ];
        if let Some(root) = &self.watch_root {
            subs.push(Subscription::run_with(root.clone(), watch_stream));
        }
        subs.push(Subscription::run_with(self.pty_output.clone(), pty_stream));
        // The screencast is driven by the window's present clock while recording:
        // `window::frames()` yields one tick per present (self-sustaining,
        // since each tick requests the next redraw), which keeps an idle window
        // presenting so screenshots resolve in real time. `on_record_frame_tick`
        // throttles these down to the configured fps.
        if self.core.is_recording() {
            subs.push(window::frames().map(Message::RecordFrameTick));
        }
        Subscription::batch(subs)
    }
}

/// Tests for the keyboard routing seam in [`Shell::on_key`]: a configured
/// shortcut must win over raw terminal input, unbound keys must reach the PTY,
/// and keys are swallowed unless a terminal holds focus. These exercise the
/// precedence wiring that the pure `termherd_pty::key_bytes` tests cannot.
#[cfg(test)]
mod key_routing {
    use super::*;
    use crate::settings::ConfirmClose;
    use iced::keyboard::key::{Named, NativeCode, Physical};
    use iced::keyboard::{Key, Location, Modifiers};
    use std::sync::Mutex as StdMutex;
    use termherd_core::SpawnSpec;
    use termherd_core::ports::{PtyError, ScanError};

    /// A `PtyHost` double recording every write and kill; all calls succeed.
    #[derive(Default)]
    struct RecordingPty {
        writes: StdMutex<Vec<Vec<u8>>>,
        kills: StdMutex<usize>,
        spawns: StdMutex<usize>,
        launches: StdMutex<Vec<Launch>>,
        resizes: StdMutex<Vec<(u16, u16)>>,
        scrolls: StdMutex<Vec<ScrollTarget>>,
    }

    impl RecordingPty {
        fn writes(&self) -> Vec<Vec<u8>> {
            self.writes.lock().expect("writes lock").clone()
        }
        fn kill_count(&self) -> usize {
            *self.kills.lock().expect("kills lock")
        }
        fn spawn_count(&self) -> usize {
            *self.spawns.lock().expect("spawns lock")
        }
        /// The launch kind of every spawn, in order — lets a test assert which
        /// button drove which kind of session (FR4a).
        fn launches(&self) -> Vec<Launch> {
            self.launches.lock().expect("launches lock").clone()
        }
        fn resizes(&self) -> Vec<(u16, u16)> {
            self.resizes.lock().expect("resizes lock").clone()
        }
        fn scrolls(&self) -> Vec<ScrollTarget> {
            self.scrolls.lock().expect("scrolls lock").clone()
        }
    }

    impl PtyHost for RecordingPty {
        fn spawn(&self, spec: SpawnSpec) -> Result<(), PtyError> {
            *self.spawns.lock().expect("spawns lock") += 1;
            self.launches
                .lock()
                .expect("launches lock")
                .push(spec.launch);
            Ok(())
        }
        fn write(&self, _session: SessionId, bytes: &[u8]) -> Result<(), PtyError> {
            self.writes
                .lock()
                .expect("writes lock")
                .push(bytes.to_vec());
            Ok(())
        }
        fn resize(&self, _: SessionId, cols: u16, rows: u16) -> Result<(), PtyError> {
            self.resizes
                .lock()
                .expect("resizes lock")
                .push((cols, rows));
            Ok(())
        }
        fn scroll(&self, _: SessionId, target: ScrollTarget) -> Result<(), PtyError> {
            self.scrolls.lock().expect("scrolls lock").push(target);
            Ok(())
        }
        fn kill(&self, _: SessionId) -> Result<(), PtyError> {
            *self.kills.lock().expect("kills lock") += 1;
            Ok(())
        }
    }

    struct EmptyScanner;
    impl ProjectScanner for EmptyScanner {
        fn scan(&self) -> Result<Vec<SessionRecord>, ScanError> {
            Ok(Vec::new())
        }
    }

    /// A `Shell` with one terminal open and focused, plus its recording PTY.
    fn shell_with_terminal() -> (Shell, Arc<RecordingPty>) {
        let pty = Arc::new(RecordingPty::default());
        let (_tx, rx) = iced::futures::channel::mpsc::unbounded::<PtyEvent>();
        let mut shell = Shell::new(
            WindowConfig::default(),
            Arc::new(EmptyScanner),
            None,
            pty.clone(),
            PtyOutput::new(rx),
            Startup {
                theme: ThemeChoice::default(),
                keymap: Keymap::defaults(),
                metadata: Overlay::default(),
                collapsed: HashSet::new(),
                record: RecordConfig::default(),
                session_limit: 0,
                font_size: 14.0,
                close: CloseSettings::default(),
            },
        );
        let _ = shell.launch("/tmp/project".to_string(), Launch::Shell);
        assert!(
            shell.core.workspace.focused_session().is_some(),
            "a launched terminal should be focused"
        );
        (shell, pty)
    }

    /// A shell whose one terminal is actively working, so a close request arms
    /// the confirmation bar rather than closing outright — the setup for tests
    /// about the confirmation machinery itself, now that an idle shell
    /// closes silently.
    fn busy_shell_with_terminal() -> (Shell, Arc<RecordingPty>) {
        let (mut shell, pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyStatus {
            session,
            status: SessionStatus::Busy,
        });
        (shell, pty)
    }

    #[test]
    fn the_claude_button_launches_a_fresh_claude_session() {
        let (mut shell, pty) = shell_with_terminal();
        let before = pty.launches().len();
        let _ = shell.update(Message::LaunchClaude("/tmp/project".to_string()));
        let launches = pty.launches();
        assert_eq!(launches.len(), before + 1, "one new spawn");
        assert_eq!(
            launches.last(),
            Some(&Launch::Claude { resume: None }),
            "the bot button starts a fresh Claude session — never a shell, never a resume"
        );
    }

    #[test]
    fn launch_buttons_title_tabs_by_kind() {
        // The initial tab label distinguishes a shell ($) from a Claude (🤖)
        // tab for the same repo; OSC retitling takes over later.
        let (mut shell, _pty) = shell_with_terminal();
        let _ = shell.update(Message::LaunchProject("/tmp/faceto".to_string()));
        let shell_tab = shell.core.workspace.focused_session().expect("focused");
        assert_eq!(
            shell.core.workspace.session_title(shell_tab),
            Some("faceto $")
        );
        let _ = shell.update(Message::LaunchClaude("/tmp/faceto".to_string()));
        let claude_tab = shell.core.workspace.focused_session().expect("focused");
        assert_eq!(
            shell.core.workspace.session_title(claude_tab),
            Some("faceto 🤖")
        );
    }

    /// Feed one browsable Claude session with a chosen name into the core, so a
    /// later resume can pick its digest title up.
    fn browse_named(shell: &mut Shell, id: &str, path: &str, summary: &str, custom: Option<&str>) {
        let record = SessionRecord {
            session_id: id.to_string(),
            project_path: path.to_string(),
            digest: termherd_claude::digest::SessionDigest {
                summary: summary.to_string(),
                message_count: 1,
                text_content: String::new(),
                slug: None,
                custom_title: custom.map(str::to_string),
                ai_title: None,
                tail: Vec::new(),
            },
            modified: None,
        };
        let _ = shell
            .core
            .apply(termherd_core::Event::ScanCompleted(vec![record]));
    }

    /// The sidebar was split into per-section row builders; render it with every
    /// section live — a starred favorite (so its section and leading divider
    /// appear), the Plans & mémoire docs, and a project group whose two rows
    /// collide on title — to prove the split assembles a valid tree across all
    /// branches without dropping or panicking on one.
    #[test]
    fn the_split_sidebar_renders_every_section() {
        let (mut shell, _pty) = shell_with_terminal();
        let row = |id: &str| SessionRecord {
            session_id: id.to_string(),
            project_path: "/tmp/alpha".to_string(),
            digest: termherd_claude::digest::SessionDigest {
                summary: "shared title".to_string(),
                message_count: 1,
                text_content: String::new(),
                slug: None,
                custom_title: None,
                ai_title: None,
                tail: Vec::new(),
            },
            modified: None,
        };
        let _ = shell.core.apply(termherd_core::Event::ScanCompleted(vec![
            row("sess-a"),
            row("sess-b"),
        ]));
        // Star one so the Favorites section — and the divider before it — shows.
        let _ = shell.update(Message::ToggleStar("sess-a".to_string()));
        assert!(
            !shell
                .core
                .favorite_sessions(&shell.core.visible_projects())
                .is_empty(),
            "a starred session should surface as a favorite",
        );
        // Populate the Plans & mémoire section.
        shell.docs = vec![DocEntry {
            kind: crate::docs::DocKind::Plan,
            label: "PRD.md".to_string(),
            path: std::path::PathBuf::from("/tmp/PRD.md"),
        }];
        // Building the whole tree must not panic across favorites + plans +
        // projects and their dividers.
        let _ = shell.view();
    }

    #[test]
    fn resuming_a_known_session_titles_the_tab_with_its_session_name() {
        // Claude (2.1.195) emits no OSC title, so the live-title override
        // never fires here — the tab must take the session's name from the
        // scanned digest instead of the generic `project 🤖` kind label.
        let (mut shell, _pty) = shell_with_terminal();
        browse_named(
            &mut shell,
            "sess",
            "/tmp/project",
            "Fix the login bug",
            None,
        );
        let _ = shell.update(Message::LaunchSession {
            cwd: "/tmp/project".to_string(),
            resume: "sess".to_string(),
        });
        let tab = shell.core.workspace.focused_session().expect("focused");
        assert_eq!(
            shell.core.workspace.session_title(tab),
            Some("Fix the login bug"),
            "a resumed tab shows the session name, not the kind label"
        );
    }

    #[test]
    fn resuming_prefers_a_custom_title_over_the_summary() {
        // The title precedence (custom > summary) must carry into the tab.
        let (mut shell, _pty) = shell_with_terminal();
        browse_named(
            &mut shell,
            "sess",
            "/tmp/project",
            "raw first prompt",
            Some("Renamed session"),
        );
        let _ = shell.update(Message::LaunchSession {
            cwd: "/tmp/project".to_string(),
            resume: "sess".to_string(),
        });
        let tab = shell.core.workspace.focused_session().expect("focused");
        assert_eq!(
            shell.core.workspace.session_title(tab),
            Some("Renamed session")
        );
    }

    #[test]
    fn an_osc_title_still_overrides_the_resumed_digest_name() {
        // The digest name is only the *initial* label. On any Claude/platform
        // that does emit an OSC title, that live title must still win —
        // guards the path deliberately left intact.
        let (mut shell, _pty) = shell_with_terminal();
        browse_named(
            &mut shell,
            "sess",
            "/tmp/project",
            "Fix the login bug",
            None,
        );
        let _ = shell.update(Message::LaunchSession {
            cwd: "/tmp/project".to_string(),
            resume: "sess".to_string(),
        });
        let tab = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyTitle {
            session: tab,
            title: "✳ refactoring".to_string(),
        });
        assert_eq!(
            shell.core.workspace.session_title(tab),
            Some("✳ refactoring"),
            "a live OSC title overrides the initial digest name"
        );
    }

    #[test]
    fn resuming_a_session_with_a_blank_name_keeps_the_kind_label() {
        // A scanned record whose digest yields an empty title must not blank the
        // tab — fall back to the kind label.
        let (mut shell, _pty) = shell_with_terminal();
        browse_named(&mut shell, "sess", "/tmp/project", "", None);
        let _ = shell.update(Message::LaunchSession {
            cwd: "/tmp/project".to_string(),
            resume: "sess".to_string(),
        });
        let tab = shell.core.workspace.focused_session().expect("focused");
        assert_eq!(shell.core.workspace.session_title(tab), Some("project 🤖"));
    }

    #[test]
    fn resuming_an_unknown_session_keeps_the_kind_label() {
        // No scanned record (a session the last scan missed) → the tab keeps the
        // cwd-derived kind label rather than an empty or wrong name. Green today;
        // guards the fix's fallback so it never regresses.
        let (mut shell, _pty) = shell_with_terminal();
        let _ = shell.update(Message::LaunchSession {
            cwd: "/tmp/ghost".to_string(),
            resume: "missing".to_string(),
        });
        let tab = shell.core.workspace.focused_session().expect("focused");
        assert_eq!(shell.core.workspace.session_title(tab), Some("ghost 🤖"));
    }

    #[test]
    fn repeated_claude_launch_opens_distinct_tabs() {
        let (mut shell, _pty) = shell_with_terminal();
        let before = shell.core.workspace.tabs.len();
        let _ = shell.update(Message::LaunchClaude("/tmp/project".to_string()));
        let _ = shell.update(Message::LaunchClaude("/tmp/project".to_string()));
        assert_eq!(
            shell.core.workspace.tabs.len(),
            before + 2,
            "fresh-Claude launches never dedupe — two clicks, two tabs"
        );
    }

    fn press(key: Key, modifiers: Modifiers, text: Option<&str>) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: Physical::Unidentified(NativeCode::Unidentified),
            location: Location::Standard,
            modifiers,
            text: text.map(Into::into),
            repeat: false,
        }
    }

    #[test]
    fn the_bundled_window_icon_decodes() {
        // Guards the icon wiring: if the bundled PNG is ever swapped for a
        // format `window_icon` can't decode, the window would silently lose its
        // icon. Fail the build instead.
        assert!(
            window_icon().is_some(),
            "the bundled 256x256.png must decode to an RGBA window icon"
        );
    }

    #[test]
    fn unbound_keys_reach_the_pty() {
        let (mut shell, pty) = shell_with_terminal();
        let _ = shell.on_key(press(
            Key::Character("a".into()),
            Modifiers::default(),
            Some("a"),
        ));
        // A modified key with no binding still falls through to its bytes.
        let _ = shell.on_key(press(Key::Named(Named::Enter), Modifiers::SHIFT, None));
        assert_eq!(pty.writes(), vec![b"a".to_vec(), b"\n".to_vec()]);
    }

    #[test]
    fn a_bound_shortcut_is_intercepted_before_the_pty() {
        let (mut shell, pty) = shell_with_terminal();
        // Ctrl+Tab is bound to NextTab on every platform; it must run the
        // action, not send the `\t` that key_bytes would otherwise produce.
        let _ = shell.on_key(press(Key::Named(Named::Tab), Modifiers::CTRL, None));
        assert!(
            pty.writes().is_empty(),
            "a bound shortcut must not write to the PTY, got {:?}",
            pty.writes()
        );
    }

    #[test]
    fn ime_commit_writes_composed_text_to_the_focused_pty() {
        // a dead-key composition (e.g. `^` then `e`) reaches the terminal
        // as the resolved character's UTF-8 bytes.
        let (mut shell, pty) = shell_with_terminal();
        let _ = shell.update(Message::ImeCommit("ê".to_string()));
        assert_eq!(pty.writes(), vec!["ê".as_bytes().to_vec()]);
    }

    #[test]
    fn ime_commit_is_ignored_without_terminal_focus() {
        // The composing overlay (search / rename) owns its own input, so a stray
        // commit must not leak into the terminal when it is not focused.
        let (mut shell, pty) = shell_with_terminal();
        shell.focus = Focus::Search;
        let _ = shell.update(Message::ImeCommit("ê".to_string()));
        assert!(pty.writes().is_empty());
    }

    #[test]
    fn ime_commit_is_ignored_while_the_archive_modal_is_up() {
        // The archive confirmation is a full-screen modal (like quit / tab-close),
        // so a composed IME character must not leak to the terminal underneath it
        // even though focus stays `Terminal`.
        let (mut shell, pty) = shell_with_terminal();
        shell.archiving = Some("sess".to_string());
        let _ = shell.update(Message::ImeCommit("ê".to_string()));
        assert!(pty.writes().is_empty());
    }

    /// Build a `Screen` of one line of text, for seeding the focused PTY of a
    /// capture test.
    fn screen_of(text: &str) -> Screen {
        let line: Vec<termherd_pty::ScreenCell> = text
            .chars()
            .map(|c| termherd_pty::ScreenCell {
                c,
                fg: [0, 0, 0],
                bg: [0, 0, 0],
                bold: false,
            })
            .collect();
        Screen {
            cols: line.len() as u16,
            rows: 1,
            lines: vec![line],
            cursor: None,
            scrolled: false,
            display_offset: 0,
            bracketed_paste: false,
        }
    }

    #[test]
    fn capture_writes_a_json_dump_with_the_focused_pty_text() {
        // a capture writes capture-<ts>.json with the focused tab, its
        // status, and the focused terminal's visible text. Driven through the
        // `perform_capture` dir seam so it lands in a tempdir, not the real home;
        // the PNG is an async iced screenshot and is not exercised here.
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        shell.screens.insert(session, screen_of("$ cargo test"));

        // Build the dump through the same seam `capture()` uses for PTY text.
        let text = shell.focused_pty_text();
        assert_eq!(text.as_deref(), Some("$ cargo test"));
        let dump = shell.core.build_capture(text);

        let dir = tempfile::tempdir().expect("tempdir");
        let _ = shell.perform_capture(dir.path(), dump);

        let written = std::fs::read_dir(dir.path())
            .expect("captures dir exists")
            .filter_map(Result::ok)
            .find(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name.starts_with("capture-") && name.ends_with(".json")
            })
            .expect("a capture-*.json was written");
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(written.path()).expect("read"))
                .expect("valid json");
        assert_eq!(json["active_tab"], 0);
        assert_eq!(json["focused_pty"], "$ cargo test");
        assert_eq!(json["tabs"][0]["title"], "project $");
        assert_eq!(json["tabs"][0]["focus_session"], session.0.get());
    }

    #[test]
    fn ime_commit_does_not_leak_into_an_inline_rename() {
        // Focus stays on the terminal while renaming inline, so a dead-key
        // composition must not reach the PTY — the rename field owns it.
        let (mut shell, pty) = shell_with_terminal();
        shell.renaming = Some(("sid".to_string(), "café".to_string()));
        let _ = shell.update(Message::ImeCommit("é".to_string()));
        assert!(pty.writes().is_empty());
    }

    #[test]
    fn clicking_elsewhere_cancels_an_inline_rename() {
        // Clicking another part of the UI while renaming (here: focusing the
        // search box) discards the in-progress edit — blur cancels.
        let (mut shell, _pty) = shell_with_terminal();
        shell.renaming = Some(("sid".to_string(), "half-typed".to_string()));
        let _ = shell.update(Message::FocusSearch);
        assert!(
            shell.renaming.is_none(),
            "a click elsewhere should cancel the rename"
        );
    }

    #[test]
    fn background_traffic_never_cancels_an_inline_rename() {
        // PTY output, key events, and the rename's own input all arrive while an
        // edit is open; none of them may discard it, or a chatty terminal would
        // make renaming impossible.
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        shell.renaming = Some(("sid".to_string(), "typing".to_string()));
        let _ = shell.update(Message::PtyStatus {
            session,
            status: SessionStatus::Busy,
        });
        let _ = shell.update(Message::RenameInput("typing more".to_string()));
        assert_eq!(
            shell.renaming.as_ref().map(|(_, b)| b.as_str()),
            Some("typing more"),
            "background and rename-internal messages must leave the edit intact"
        );
    }

    #[test]
    fn ime_commit_is_swallowed_by_a_pending_close_confirmation() {
        // A close confirmation captures input; an IME commit must not slip
        // past it to the terminal even though focus is still on it.
        let (mut shell, pty) = busy_shell_with_terminal();
        let _ = shell.update(Message::RequestCloseTab(0));
        let _ = shell.update(Message::ImeCommit("ê".to_string()));
        assert!(pty.writes().is_empty());
    }

    #[test]
    fn keys_are_ignored_without_terminal_focus() {
        let (mut shell, pty) = shell_with_terminal();
        shell.focus = Focus::Search;
        let _ = shell.on_key(press(
            Key::Character("a".into()),
            Modifiers::default(),
            Some("a"),
        ));
        assert!(pty.writes().is_empty());
    }

    #[test]
    fn requesting_a_close_only_arms_it_confirming_kills() {
        let (mut shell, pty) = busy_shell_with_terminal();
        // Clicking the tab's × arms the confirmation but kills nothing.
        let _ = shell.update(Message::RequestCloseTab(0));
        assert_eq!(shell.closing, Some(0));
        assert_eq!(pty.kill_count(), 0, "arming must not kill the session");
        // Accepting the confirmation kills it and clears the pending state.
        let _ = shell.update(Message::CloseTab(0));
        assert_eq!(pty.kill_count(), 1);
        assert_eq!(shell.closing, None);
    }

    #[test]
    fn cancelling_a_close_leaves_the_session_alive() {
        let (mut shell, pty) = busy_shell_with_terminal();
        let _ = shell.update(Message::RequestCloseTab(0));
        let _ = shell.update(Message::CancelClose);
        assert_eq!(shell.closing, None);
        assert_eq!(pty.kill_count(), 0);
    }

    #[test]
    fn a_no_confirmation_tab_policy_closes_without_arming() {
        let (mut shell, pty) = shell_with_terminal();
        shell.close_confirm.tab = ConfirmClose::NoConfirmation;
        let _ = shell.update(Message::RequestCloseTab(0));
        assert!(shell.closing.is_none(), "noConfirmation never arms the bar");
        assert_eq!(pty.kill_count(), 1, "the session is killed straight away");
    }

    #[test]
    fn a_confirm_when_active_tab_prompts_while_running_then_skips_once_exited() {
        // Under `confirmWhenActive` (the default), the prompt keys off the core
        // `tab_has_running_process` predicate: a working shell confirms…
        let (mut shell, _pty) = busy_shell_with_terminal();
        shell.close_confirm.tab = ConfirmClose::ConfirmWhenActive;
        let _ = shell.update(Message::RequestCloseTab(0));
        assert_eq!(shell.closing, Some(0), "a running tab confirms");
        let _ = shell.update(Message::CancelClose);
        // …but once its session has exited, the close needs no prompt.
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyExited(session));
        let _ = shell.update(Message::RequestCloseTab(0));
        assert!(
            shell.closing.is_none(),
            "an exited tab closes without a prompt"
        );
        assert!(
            shell.core.workspace.tabs.is_empty(),
            "the tab is gone after the unprompted close"
        );
    }

    /// The first session id hosted by each tab, in tab order — a stable handle
    /// to assert reordering against.
    fn tab_order(shell: &Shell) -> Vec<SessionId> {
        shell
            .core
            .workspace
            .tabs
            .iter()
            .map(|t| t.sessions()[0])
            .collect()
    }

    /// A shell with three open tabs (the launched terminal plus two more).
    fn shell_with_three_tabs() -> Shell {
        let (mut shell, _pty) = shell_with_terminal();
        let _ = shell.launch("/tmp/b".to_string(), Launch::Shell);
        let _ = shell.launch("/tmp/c".to_string(), Launch::Shell);
        assert_eq!(shell.core.workspace.tabs.len(), 3);
        shell
    }

    #[test]
    fn confirmations_route_through_one_modal_in_priority_order() {
        // Quit, tab-close and archive all confirm via the same modal, and at
        // most one shows at a time — quit > close > archive.
        let mut shell = shell_with_three_tabs();
        assert!(
            shell.active_confirmation().is_none(),
            "nothing armed → no modal"
        );

        shell.closing = Some(0);
        assert!(
            matches!(shell.active_confirmation(), Some((_, Message::CancelClose))),
            "a tab close arms the close modal"
        );

        shell.closing = None;
        shell.archiving = Some("sess".to_string());
        assert!(
            matches!(
                shell.active_confirmation(),
                Some((_, Message::CancelArchive))
            ),
            "an archive alone arms the archive modal"
        );

        // Armed together, quit outranks the tab close (and the archive).
        shell.closing = Some(0);
        shell.closing_window = Some(window::Id::unique());
        assert!(
            matches!(
                shell.active_confirmation(),
                Some((_, Message::CancelCloseWindow))
            ),
            "quit takes precedence over the other confirmations"
        );
    }

    #[test]
    fn double_clicking_a_tab_then_typing_and_enter_renames_it() {
        let mut shell = shell_with_three_tabs();
        let derived = shell.core.workspace.tabs[1].display_title().to_owned();

        let _ = shell.update(Message::StartTabRename {
            index: 1,
            current: derived.clone(),
        });
        // The edit anchors on tab 1's session, so it resolves back to index 1.
        let anchor = shell
            .tab_rename
            .as_ref()
            .map(|(a, _)| *a)
            .expect("renaming");
        assert_eq!(shell.core.workspace.tab_of_session(anchor), Some(1));

        let _ = shell.update(Message::TabRenameInput("My work".to_string()));
        let _ = shell.update(Message::CommitTabRename);

        assert_eq!(shell.core.workspace.tabs[1].display_title(), "My work");
        assert!(shell.tab_rename.is_none(), "committing clears the editor");
    }

    #[test]
    fn escape_abandons_a_tab_rename_without_touching_the_title() {
        let mut shell = shell_with_three_tabs();
        let derived = shell.core.workspace.tabs[1].display_title().to_owned();

        let _ = shell.update(Message::StartTabRename {
            index: 1,
            current: derived.clone(),
        });
        let _ = shell.update(Message::TabRenameInput("half-typed".to_string()));
        let _ = shell.on_key(press(Key::Named(Named::Escape), Modifiers::default(), None));

        assert!(shell.tab_rename.is_none(), "Escape abandons the edit");
        assert_eq!(
            shell.core.workspace.tabs[1].display_title(),
            derived,
            "an abandoned rename leaves the derived title intact"
        );
    }

    #[test]
    fn pressing_another_tab_commits_the_rename_but_the_double_clicks_own_drag_does_not() {
        let mut shell = shell_with_three_tabs();
        let derived = shell.core.workspace.tabs[1].display_title().to_owned();

        let _ = shell.update(Message::StartTabRename {
            index: 1,
            current: derived,
        });
        let _ = shell.update(Message::TabRenameInput("Renamed".to_string()));

        // The double-click that opened the edit still emits TabDragStart(1) /
        // TabDragEnd around it — those must not commit or the field would vanish
        // before a key is pressed.
        let _ = shell.update(Message::TabDragStart(1));
        let _ = shell.update(Message::TabDragEnd);
        assert!(
            shell.tab_rename.is_some(),
            "the renamed tab's own drag noise leaves the edit open"
        );

        // A press on a *different* tab is a real blur → commit.
        let _ = shell.update(Message::TabDragStart(0));
        assert!(shell.tab_rename.is_none(), "clicking another tab commits");
        assert_eq!(shell.core.workspace.tabs[1].display_title(), "Renamed");
    }

    #[test]
    fn committing_a_blank_tab_rename_reverts_to_the_derived_title() {
        let mut shell = shell_with_three_tabs();
        let derived = shell.core.workspace.tabs[1].display_title().to_owned();

        let _ = shell.update(Message::StartTabRename {
            index: 1,
            current: derived.clone(),
        });
        let _ = shell.update(Message::TabRenameInput("   ".to_string()));
        let _ = shell.update(Message::CommitTabRename);

        assert_eq!(
            shell.core.workspace.tabs[1].display_title(),
            derived,
            "a blank rename falls back to the derived title"
        );
    }

    #[test]
    fn committing_an_unchanged_tab_name_leaves_the_title_dynamic() {
        let mut shell = shell_with_three_tabs();
        let derived = shell.core.workspace.tabs[1].display_title().to_owned();

        // Open the editor (seeded with the shown title) and commit without
        // editing — an accidental double-click + Enter.
        let _ = shell.update(Message::StartTabRename {
            index: 1,
            current: derived,
        });
        let _ = shell.update(Message::CommitTabRename);

        // No override is stored, so the tab keeps tracking its derived title
        // rather than freezing the current one against future relabels.
        assert!(
            shell.core.workspace.tabs[1].custom_title.is_none(),
            "an unchanged commit must not create an override"
        );
    }

    #[test]
    fn a_genuine_interaction_elsewhere_commits_a_pending_tab_rename() {
        let mut shell = shell_with_three_tabs();
        let derived = shell.core.workspace.tabs[1].display_title().to_owned();

        let _ = shell.update(Message::StartTabRename {
            index: 1,
            current: derived,
        });
        let _ = shell.update(Message::TabRenameInput("Renamed".to_string()));

        // Starring a sidebar session is a real blur — it dismisses a session
        // rename, so it must also commit a tab rename (shared allowlist).
        let _ = shell.update(Message::ToggleStar("sess".to_string()));

        assert!(shell.tab_rename.is_none(), "an elsewhere-click commits");
        assert_eq!(shell.core.workspace.tabs[1].display_title(), "Renamed");
    }

    #[test]
    fn a_pending_tab_rename_follows_its_tab_across_a_reorder() {
        let mut shell = shell_with_three_tabs();
        let derived = shell.core.workspace.tabs[2].display_title().to_owned();
        let _ = shell.update(Message::StartTabRename {
            index: 2,
            current: derived,
        });
        let _ = shell.update(Message::TabRenameInput("Pinned".to_string()));
        let anchor = shell
            .tab_rename
            .as_ref()
            .map(|(a, _)| *a)
            .expect("renaming");

        // A reorder shifts the anchored tab to a new index without committing.
        // Because the edit anchors on the session, not the position, the commit
        // must still land on the right tab.
        let _ = shell
            .core
            .apply(termherd_core::Event::MoveTab { from: 0, to: 2 });
        let _ = shell.update(Message::CommitTabRename);

        let idx = shell
            .core
            .workspace
            .tab_of_session(anchor)
            .expect("the anchored tab still exists");
        assert_eq!(shell.core.workspace.tabs[idx].display_title(), "Pinned");
        let renamed = shell
            .core
            .workspace
            .tabs
            .iter()
            .filter(|t| t.display_title() == "Pinned")
            .count();
        assert_eq!(renamed, 1, "only the anchored tab is renamed");
    }

    #[test]
    fn dragging_a_tab_reorders_the_workspace() {
        let mut shell = shell_with_three_tabs();
        let before = tab_order(&shell);
        // Press tab 0, drag across onto tab 2's slot, release.
        let _ = shell.update(Message::TabDragStart(0));
        let _ = shell.update(Message::TabDragOver(1));
        let _ = shell.update(Message::TabDragOver(2));
        let _ = shell.update(Message::TabDragEnd);
        assert_eq!(tab_order(&shell), vec![before[1], before[2], before[0]]);
        assert!(shell.tab_drag.is_none(), "the drag is cleared on release");
    }

    #[test]
    fn a_plain_tab_click_activates_without_reordering() {
        let mut shell = shell_with_three_tabs(); // active is the last tab (2)
        let before = tab_order(&shell);
        // Press and release on tab 0 with no hover onto another tab — a click.
        let _ = shell.update(Message::TabDragStart(0));
        let _ = shell.update(Message::TabDragEnd);
        assert_eq!(tab_order(&shell), before, "a click must not reorder");
        assert_eq!(shell.core.workspace.active, 0, "the clicked tab is active");
        assert!(shell.tab_drag.is_none());
    }

    #[test]
    fn leaving_the_strip_abandons_a_drag() {
        let mut shell = shell_with_three_tabs();
        let before = tab_order(&shell);
        let active_before = shell.core.workspace.active;
        let _ = shell.update(Message::TabDragStart(0));
        let _ = shell.update(Message::TabDragOver(2));
        let _ = shell.update(Message::TabDragCancel);
        // A release that arrives after the cancel finds no drag and does nothing.
        let _ = shell.update(Message::TabDragEnd);
        assert_eq!(
            tab_order(&shell),
            before,
            "an abandoned drag changes nothing"
        );
        assert_eq!(shell.core.workspace.active, active_before);
        assert!(shell.tab_drag.is_none());
    }

    #[test]
    fn the_confirmation_owns_the_keyboard() {
        // Escape dismisses the prompt without killing.
        let (mut shell, pty) = busy_shell_with_terminal();
        let _ = shell.update(Message::RequestCloseTab(0));
        let _ = shell.on_key(press(Key::Named(Named::Escape), Modifiers::default(), None));
        assert_eq!(shell.closing, None);
        assert_eq!(pty.kill_count(), 0);

        // Enter confirms; meanwhile a plain key is swallowed, not sent.
        let (mut shell, pty) = busy_shell_with_terminal();
        let _ = shell.update(Message::RequestCloseTab(0));
        let _ = shell.on_key(press(
            Key::Character("a".into()),
            Modifiers::default(),
            Some("a"),
        ));
        assert!(
            pty.writes().is_empty(),
            "keys must not reach the PTY mid-confirm"
        );
        let _ = shell.on_key(press(Key::Named(Named::Enter), Modifiers::default(), None));
        assert_eq!(pty.kill_count(), 1);
    }

    #[test]
    fn an_out_of_range_close_request_is_ignored() {
        let (mut shell, _pty) = shell_with_terminal();
        let _ = shell.update(Message::RequestCloseTab(7));
        assert_eq!(shell.closing, None, "a stale index must not arm a close");
    }

    #[test]
    fn closing_an_idle_shell_tab_skips_the_confirmation() {
        // A plain shell parked at its prompt has nothing to lose, so a close
        // must take effect immediately — no confirmation bar, and the session
        // is actually killed.
        let (mut shell, pty) = shell_with_terminal();
        let _ = shell.update(Message::RequestCloseTab(0));
        assert_eq!(shell.closing, None, "an idle shell needs no confirmation");
        assert_eq!(pty.kill_count(), 1, "the tab closes there and then");
        assert!(shell.core.workspace.tabs.is_empty(), "the tab is gone");
    }

    #[test]
    fn closing_a_busy_shell_tab_still_confirms() {
        // Once the shell is working, the same close must arm the confirmation
        // instead of killing outright.
        let (mut shell, pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyStatus {
            session,
            status: SessionStatus::Busy,
        });
        let _ = shell.update(Message::RequestCloseTab(0));
        assert_eq!(shell.closing, Some(0), "a busy shell arms a confirmation");
        assert_eq!(pty.kill_count(), 0, "arming must not kill the session");
    }

    #[test]
    fn closing_a_claude_tab_always_confirms() {
        // A Claude session is a running foreground process even when idle, so
        // its tab must always confirm before closing.
        let (mut shell, pty) = shell_with_terminal();
        let _ = shell.launch("/tmp/claude".to_string(), Launch::Claude { resume: None });
        let claude_tab = shell.core.workspace.active;
        let _ = shell.update(Message::RequestCloseTab(claude_tab));
        assert_eq!(shell.closing, Some(claude_tab), "a Claude tab confirms");
        assert_eq!(pty.kill_count(), 0, "arming must not kill the session");
    }

    #[test]
    fn an_armed_confirmation_ignores_a_close_request_for_another_tab() {
        // While a close confirmation is up on the busy tab 0, clicking a *second*
        // (idle) tab's × must not silently close it — and above all must not drop
        // the pending confirmation. The prompt owns the interaction, like the
        // keyboard does, until it is answered or cancelled.
        let (mut shell, pty) = busy_shell_with_terminal();
        let _ = shell.launch("/tmp/idle".to_string(), Launch::Shell);
        assert_eq!(shell.core.workspace.tabs.len(), 2);
        let _ = shell.update(Message::RequestCloseTab(0));
        assert_eq!(shell.closing, Some(0), "the busy tab arms the confirmation");

        let _ = shell.update(Message::RequestCloseTab(1));
        assert_eq!(shell.closing, Some(0), "the armed confirmation stays put");
        assert_eq!(shell.core.workspace.tabs.len(), 2, "no tab was closed");
        assert_eq!(
            pty.kill_count(),
            0,
            "nothing is killed while a prompt is up"
        );
    }

    #[test]
    fn collapsing_the_sidebar_widens_the_grid_and_resizes_the_pty() {
        // hiding the sidebar must grow the column count (the reclaimed
        // width becomes columns), and the toggle must push that wider size to
        // the PTY rather than leaving cols stale (which stretched the cells).
        let (mut shell, pty) = shell_with_terminal();
        let (cols_visible, _) = shell.grid_size();
        let resizes_before = pty.resizes().len();

        let _ = shell.toggle_sidebar();
        assert!(shell.core.sidebar_hidden, "toggle should hide the sidebar");

        let (cols_hidden, _) = shell.grid_size();
        assert!(
            cols_hidden > cols_visible,
            "hiding the sidebar must add columns (was {cols_visible}, now {cols_hidden})"
        );
        let resizes = pty.resizes();
        assert!(
            resizes.len() > resizes_before,
            "toggling the sidebar must resize the focused PTY"
        );
        assert_eq!(
            resizes.last().map(|(cols, _)| *cols),
            Some(cols_hidden),
            "the resize must carry the new, wider column count"
        );
    }

    #[test]
    fn scroll_top_and_bottom_actions_jump_the_focused_viewport() {
        // the scroll-top/bottom shortcuts send an absolute jump to the
        // focused session's PTY, through the same path as the mouse wheel.
        let (mut shell, pty) = shell_with_terminal();
        let _ = shell.run_action(Action::ScrollTop);
        let _ = shell.run_action(Action::ScrollBottom);
        // The wheel shares the path and lands a wheel turn at the pointer cell,
        // routed to the session under the pointer.
        let session = shell
            .core
            .workspace
            .focused_session()
            .expect("a launched terminal is focused");
        let _ = shell.update(Message::TermScroll {
            session,
            col: 0,
            row: 0,
            lines: 3,
        });
        assert_eq!(
            pty.scrolls(),
            vec![
                ScrollTarget::Top,
                ScrollTarget::Bottom,
                ScrollTarget::Wheel {
                    col: 0,
                    row: 0,
                    lines: 3
                }
            ]
        );
    }

    /// A `Shell` with no terminal open (empty workspace), plus its recording
    /// PTY — for the "new shell here" empty-workspace path.
    fn empty_shell() -> (Shell, Arc<RecordingPty>) {
        let pty = Arc::new(RecordingPty::default());
        let (_tx, rx) = iced::futures::channel::mpsc::unbounded::<PtyEvent>();
        let shell = Shell::new(
            WindowConfig::default(),
            Arc::new(EmptyScanner),
            None,
            pty.clone(),
            PtyOutput::new(rx),
            Startup {
                theme: ThemeChoice::default(),
                keymap: Keymap::defaults(),
                metadata: Overlay::default(),
                collapsed: HashSet::new(),
                record: RecordConfig::default(),
                session_limit: 0,
                font_size: 14.0,
                close: CloseSettings::default(),
            },
        );
        assert!(shell.core.workspace.focused_session().is_none());
        (shell, pty)
    }

    /// The cwd registered for the currently focused session, for asserting which
    /// directory a context launch landed in.
    fn focused_cwd(shell: &Shell) -> Option<String> {
        let id = shell.core.workspace.focused_session()?;
        shell.core.sessions.get(&id)?.cwd.clone()
    }

    #[test]
    fn new_shell_here_inherits_the_focused_directory() {
        // mod+T opens a shell in the focused session's cwd.
        let (mut shell, pty) = shell_with_terminal();
        let before = pty.spawn_count();
        let _ = shell.run_action(Action::NewShellHere);
        assert_eq!(pty.spawn_count(), before + 1, "one new shell spawned");
        assert_eq!(pty.launches().last(), Some(&Launch::Shell));
        assert_eq!(focused_cwd(&shell).as_deref(), Some("/tmp/project"));
    }

    #[test]
    fn new_shell_here_falls_back_to_home_on_an_empty_workspace() {
        // with nothing open, mod+T still opens a shell — in the home dir.
        let (mut shell, pty) = empty_shell();
        let _ = shell.run_action(Action::NewShellHere);
        assert_eq!(pty.spawn_count(), 1, "a shell opens even with no context");
        assert_eq!(pty.launches().last(), Some(&Launch::Shell));
        assert_eq!(
            focused_cwd(&shell).as_deref(),
            Some(home_dir().as_str()),
            "the empty-workspace shell lands in the home directory"
        );
    }

    #[test]
    fn new_claude_here_launches_claude_in_the_focused_context() {
        // mod+Alt+T starts a fresh Claude session anchored on the focused
        // context. With no `.git` above the cwd, repo_root falls back to it.
        let (mut shell, pty) = shell_with_terminal();
        let before = pty.spawn_count();
        let _ = shell.run_action(Action::NewClaudeSessionHere);
        assert_eq!(pty.spawn_count(), before + 1);
        assert_eq!(
            pty.launches().last(),
            Some(&Launch::Claude { resume: None })
        );
        assert_eq!(focused_cwd(&shell).as_deref(), Some("/tmp/project"));
    }

    #[test]
    fn new_claude_here_is_inert_without_a_context() {
        // the Claude variant has nothing to anchor on in an empty
        // workspace, so it does nothing (unlike the shell variant).
        let (mut shell, pty) = empty_shell();
        let _ = shell.run_action(Action::NewClaudeSessionHere);
        assert_eq!(pty.spawn_count(), 0, "no repo to derive — no launch");
    }

    #[test]
    fn reopen_closed_tab_restores_the_last_close_and_then_drains() {
        // close a tab, mod+Shift+T brings it back; a second reopen with an
        // empty stack does nothing.
        let (mut shell, pty) = shell_with_terminal();
        let _ = shell.update(Message::CloseTab(0));
        assert!(shell.core.workspace.tabs.is_empty());
        let spawns_before = pty.spawn_count();

        let _ = shell.run_action(Action::ReopenClosedTab);
        assert_eq!(pty.spawn_count(), spawns_before + 1, "the tab comes back");
        assert_eq!(shell.core.workspace.tabs.len(), 1);
        assert_eq!(focused_cwd(&shell).as_deref(), Some("/tmp/project"));

        let spawns_after_reopen = pty.spawn_count();
        let _ = shell.run_action(Action::ReopenClosedTab);
        assert_eq!(
            pty.spawn_count(),
            spawns_after_reopen,
            "a second reopen with an empty stack is a no-op"
        );
    }

    #[test]
    fn command_chords_fire_without_terminal_focus() {
        // the chord dispatch runs before the terminal-focus guard, so the
        // very first shell can be opened by keyboard from the empty, search-
        // focused workspace. mod+T = Cmd+T on macOS, Ctrl+T elsewhere.
        let primary = if cfg!(target_os = "macos") {
            Modifiers::LOGO
        } else {
            Modifiers::CTRL
        };
        let (mut shell, pty) = empty_shell();
        assert_eq!(
            shell.focus,
            Focus::Search,
            "an empty workspace starts on search"
        );
        let _ = shell.on_key(press(Key::Character("t".into()), primary, Some("t")));
        assert_eq!(
            pty.spawn_count(),
            1,
            "mod+T opened a shell despite no terminal focus"
        );
        assert_eq!(pty.launches().last(), Some(&Launch::Shell));
    }

    #[test]
    fn live_session_count_excludes_exited_sessions() {
        let (mut shell, _pty) = shell_with_terminal();
        assert_eq!(shell.live_session_count(), 1, "a launched session is live");
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyExited(session));
        assert_eq!(
            shell.live_session_count(),
            0,
            "an exited session no longer counts as live to kill"
        );
    }

    #[test]
    fn the_quit_modal_owns_the_keyboard() {
        // While the quit modal is up, a plain key is swallowed (not sent to the
        // terminal) and Escape dismisses it without quitting.
        let (mut shell, pty) = shell_with_terminal();
        shell.closing_window = Some(window::Id::unique());
        let _ = shell.on_key(press(
            Key::Character("a".into()),
            Modifiers::default(),
            Some("a"),
        ));
        assert!(
            pty.writes().is_empty(),
            "keys must not reach the PTY while the quit modal is up"
        );
        let _ = shell.on_key(press(Key::Named(Named::Escape), Modifiers::default(), None));
        assert!(!shell.quit_pending(), "Escape must dismiss the quit modal");
    }

    #[test]
    fn cancelling_the_quit_keeps_the_app_running_and_confirming_consumes_it() {
        let (mut shell, pty) = shell_with_terminal();

        shell.closing_window = Some(window::Id::unique());
        let _ = shell.update(Message::CancelCloseWindow);
        assert!(!shell.quit_pending(), "cancel clears the pending quit");
        assert_eq!(pty.kill_count(), 0, "cancelling kills nothing");

        // Confirming consumes the pending id (it drives an iced::exit task).
        shell.closing_window = Some(window::Id::unique());
        let _ = shell.update(Message::ConfirmCloseWindow);
        assert!(
            shell.closing_window.is_none(),
            "confirming consumes the pending window id"
        );
    }

    #[test]
    fn closing_with_no_live_sessions_terminates_the_runtime() {
        // with nothing running, Cmd+Q (a CloseRequested on macOS) must
        // actually terminate the iced runtime — not merely close the window and
        // leave the process, holding the single-instance lock, behind.
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyExited(session));
        assert_eq!(shell.live_session_count(), 0, "precondition: nothing live");

        let _ = shell.update(Message::Window(
            window::Id::unique(),
            window::Event::CloseRequested,
        ));
        assert!(
            shell.exiting,
            "a quit with no live sessions must terminate the runtime, not just the window"
        );
        assert!(
            !shell.quit_pending(),
            "no confirmation modal when nothing is running"
        );
    }

    #[test]
    fn closing_with_running_sessions_confirms_before_exiting() {
        // A running session would be hard-killed, so the first CloseRequested
        // arms the modal instead of exiting — the runtime stays up until
        // confirmed. Under the default `confirmWhenActive` an idle shell would
        // quit silently, so this needs a session that is actually working.
        let (mut shell, _pty) = busy_shell_with_terminal();
        assert!(
            shell.core.any_running_process(),
            "precondition: a session is running"
        );

        let _ = shell.update(Message::Window(
            window::Id::unique(),
            window::Event::CloseRequested,
        ));
        assert!(
            shell.quit_pending(),
            "a running session arms the quit modal"
        );
        assert!(
            !shell.exiting,
            "the runtime must not terminate before the quit is confirmed"
        );
    }

    #[test]
    fn an_idle_but_live_session_quits_silently_under_the_default() {
        // The headline of the running-process quit gate: a session parked at
        // its prompt (live, but not running foreground work) does *not* arm the
        // modal under the default `confirmWhenActive` — the app quits straight
        // away. Guards against a regression that re-nags on every open session.
        let (mut shell, _pty) = shell_with_terminal();
        assert!(
            !shell.core.any_running_process(),
            "precondition: the launched shell is idle, nothing running"
        );
        assert_eq!(
            shell.live_session_count(),
            1,
            "…but it is still a live session"
        );
        let _ = shell.update(Message::Window(
            window::Id::unique(),
            window::Event::CloseRequested,
        ));
        assert!(shell.exiting, "an all-idle app quits without a prompt");
        assert!(!shell.quit_pending(), "no modal when nothing is running");
    }

    #[test]
    fn an_always_confirm_app_policy_prompts_even_with_nothing_running() {
        let (mut shell, _pty) = shell_with_terminal();
        shell.close_confirm.app = ConfirmClose::AlwaysConfirm;
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyExited(session));
        assert_eq!(shell.live_session_count(), 0, "precondition: nothing live");
        let _ = shell.update(Message::Window(
            window::Id::unique(),
            window::Event::CloseRequested,
        ));
        assert!(
            shell.quit_pending(),
            "alwaysConfirm prompts even with nothing to hard-kill"
        );
        assert!(!shell.exiting, "the prompt holds the runtime up");
    }

    #[test]
    fn a_no_confirmation_app_policy_quits_past_running_sessions() {
        // A running session would confirm under the default; `noConfirmation`
        // quits straight through it — so use a busy shell to prove the override.
        let (mut shell, _pty) = busy_shell_with_terminal();
        shell.close_confirm.app = ConfirmClose::NoConfirmation;
        assert!(
            shell.core.any_running_process(),
            "precondition: a session is running"
        );
        let _ = shell.update(Message::Window(
            window::Id::unique(),
            window::Event::CloseRequested,
        ));
        assert!(shell.exiting, "noConfirmation quits without a modal");
        assert!(!shell.quit_pending());
    }

    #[test]
    fn confirming_the_quit_terminates_the_runtime() {
        // Accepting the modal must reach `iced::exit`, not just `window::close`
        // — that distinction is the whole point.
        let (mut shell, _pty) = shell_with_terminal();
        shell.closing_window = Some(window::Id::unique());
        let _ = shell.update(Message::ConfirmCloseWindow);
        assert!(
            shell.exiting,
            "confirming the quit must terminate the runtime"
        );
        assert!(
            shell.closing_window.is_none(),
            "confirming consumes the pending quit"
        );
    }

    #[test]
    fn cmd_q_routes_through_the_same_seam_as_the_close_button() {
        // On macOS the menu Quit item (and Cmd+Q) is repointed to
        // `performClose:`, so it reaches the runtime as the *same*
        // `CloseRequested` window event the close button produces. That native
        // repoint can't be exercised headlessly. What this test *can* pin is the
        // shared destination: both the close-button event and a direct
        // `request_quit` arm the confirm modal identically for a live session.
        // It guards `request_quit`'s confirm behaviour and that `CloseRequested`
        // routes into it — it does not, and cannot, prove some *other* future
        // code path won't bypass `request_quit`; keeping that single seam is a
        // design rule, not something this test enforces.
        let (mut shell, _pty) = busy_shell_with_terminal();
        assert!(
            shell.core.any_running_process(),
            "precondition: a session is running"
        );

        let close_button = shell.update(Message::Window(
            window::Id::unique(),
            window::Event::CloseRequested,
        ));
        assert!(shell.quit_pending(), "the close button arms the modal");
        drop(close_button);
        shell.closing_window = None;

        // The macOS menu Quit / Cmd+Q path lands on the identical seam.
        let _ = shell.request_quit(window::Id::unique());
        assert!(
            shell.quit_pending(),
            "Cmd+Q (via performClose: → CloseRequested → request_quit) must arm \
             the same modal, never bypass it"
        );
        assert!(
            !shell.exiting,
            "a live session must not be hard-killed without confirmation"
        );
    }

    /// Feed one browsable session into the shell's core so the archive flow
    /// has something to act on.
    fn browse_one(shell: &mut Shell, id: &str) {
        let record = SessionRecord {
            session_id: id.to_string(),
            project_path: "/tmp/project".to_string(),
            digest: termherd_claude::digest::SessionDigest {
                summary: "a session".to_string(),
                message_count: 1,
                text_content: String::new(),
                slug: None,
                custom_title: None,
                ai_title: None,
                tail: Vec::new(),
            },
            modified: None,
        };
        let _ = shell
            .core
            .apply(termherd_core::Event::ScanCompleted(vec![record]));
    }

    #[test]
    fn requesting_an_archive_only_arms_it_confirming_archives() {
        let (mut shell, _pty) = shell_with_terminal();
        browse_one(&mut shell, "sess");
        // Clicking the archive control arms the confirmation but archives nothing.
        let _ = shell.update(Message::RequestArchive("sess".into()));
        assert_eq!(shell.archiving.as_deref(), Some("sess"));
        assert!(
            !shell.core.is_archived("sess"),
            "arming must not archive the session"
        );
        // Accepting the confirmation archives it and clears the pending state.
        let _ = shell.update(Message::ConfirmArchive);
        assert!(shell.core.is_archived("sess"));
        assert_eq!(shell.archiving, None);
    }

    #[test]
    fn cancelling_an_archive_leaves_the_session_unarchived() {
        let (mut shell, _pty) = shell_with_terminal();
        browse_one(&mut shell, "sess");
        let _ = shell.update(Message::RequestArchive("sess".into()));
        let _ = shell.update(Message::CancelArchive);
        assert_eq!(shell.archiving, None);
        assert!(!shell.core.is_archived("sess"));
    }

    #[test]
    fn un_archiving_stays_one_click() {
        let (mut shell, _pty) = shell_with_terminal();
        browse_one(&mut shell, "sess");
        // Archive directly via the core to set up an archived session.
        let _ = shell.update(Message::RequestArchive("sess".into()));
        let _ = shell.update(Message::ConfirmArchive);
        assert!(shell.core.is_archived("sess"));
        // The un-archive path is a plain toggle with no confirmation.
        let _ = shell.update(Message::ToggleArchive("sess".into()));
        assert!(!shell.core.is_archived("sess"));
        assert_eq!(shell.archiving, None);
    }

    #[test]
    fn the_archive_confirmation_owns_the_keyboard() {
        // Escape dismisses the prompt without archiving.
        let (mut shell, _pty) = shell_with_terminal();
        browse_one(&mut shell, "sess");
        let _ = shell.update(Message::RequestArchive("sess".into()));
        let _ = shell.on_key(press(Key::Named(Named::Escape), Modifiers::default(), None));
        assert_eq!(shell.archiving, None);
        assert!(!shell.core.is_archived("sess"));

        // Enter confirms; meanwhile a plain key is swallowed, not sent.
        let (mut shell, pty) = shell_with_terminal();
        browse_one(&mut shell, "sess");
        let _ = shell.update(Message::RequestArchive("sess".into()));
        let _ = shell.on_key(press(
            Key::Character("a".into()),
            Modifiers::default(),
            Some("a"),
        ));
        assert!(
            pty.writes().is_empty(),
            "keys must not reach the PTY mid-confirm"
        );
        let _ = shell.on_key(press(Key::Named(Named::Enter), Modifiers::default(), None));
        assert!(shell.core.is_archived("sess"));
        assert_eq!(shell.archiving, None);
    }

    #[test]
    fn launching_a_session_drops_a_pending_archive() {
        // Arming an archive then opening a terminal must clear the prompt, so a
        // later Enter goes to the PTY instead of confirming the stale archive.
        let (mut shell, _pty) = shell_with_terminal();
        browse_one(&mut shell, "sess");
        let _ = shell.update(Message::RequestArchive("sess".into()));
        let _ = shell.launch("/tmp/project".to_string(), Launch::Shell);
        assert_eq!(shell.archiving, None);
        let _ = shell.on_key(press(Key::Named(Named::Enter), Modifiers::default(), None));
        assert!(
            !shell.core.is_archived("sess"),
            "a terminal Enter must not confirm a dropped archive prompt"
        );
    }

    #[test]
    fn reclicking_an_open_session_refocuses_its_tab_without_respawning() {
        // Open session "sess" in its own tab, then open a second tab so it is no
        // longer active. Re-clicking "sess" in the sidebar must bring its
        // existing tab forward, not spawn a third terminal.
        let (mut shell, pty) = shell_with_terminal();
        let _ = shell.launch(
            "/tmp/project".to_string(),
            Launch::Claude {
                resume: Some("sess".to_string()),
            },
        );
        let sess_tab = shell.core.workspace.active;
        let _ = shell.launch(
            "/tmp/other".to_string(),
            Launch::Claude {
                resume: Some("other".to_string()),
            },
        );
        assert_ne!(
            shell.core.workspace.active, sess_tab,
            "second tab is active"
        );
        let spawns_before = pty.spawn_count();
        let tabs_before = shell.core.workspace.tabs.len();

        let _ = shell.update(Message::LaunchSession {
            cwd: "/tmp/project".to_string(),
            resume: "sess".to_string(),
        });
        assert_eq!(
            shell.core.workspace.active, sess_tab,
            "re-click should re-focus the existing tab"
        );
        assert_eq!(
            pty.spawn_count(),
            spawns_before,
            "no new terminal must be spawned"
        );
        assert_eq!(shell.core.workspace.tabs.len(), tabs_before, "no new tab");
    }

    #[test]
    fn toggling_collapse_folds_and_unfolds_a_project() {
        // The sidebar's disclosure triangle routes through this message; one
        // click folds the project, a second unfolds it.
        let (mut shell, _pty) = shell_with_terminal();
        browse_one(&mut shell, "sess");
        assert!(!shell.core.is_collapsed("/tmp/project"));
        let _ = shell.update(Message::ToggleCollapsed("/tmp/project".into()));
        assert!(shell.core.is_collapsed("/tmp/project"));
        let _ = shell.update(Message::ToggleCollapsed("/tmp/project".into()));
        assert!(!shell.core.is_collapsed("/tmp/project"));
    }

    #[test]
    fn confirming_a_vanished_session_archives_nothing() {
        // A rescan can drop the armed session while the prompt is up; confirming
        // then must not persist phantom archived metadata for it.
        let (mut shell, _pty) = shell_with_terminal();
        browse_one(&mut shell, "sess");
        let _ = shell.update(Message::RequestArchive("sess".into()));
        let _ = shell
            .core
            .apply(termherd_core::Event::ScanCompleted(Vec::new()));
        let _ = shell.update(Message::ConfirmArchive);
        assert!(!shell.core.is_archived("sess"));
        assert_eq!(shell.archiving, None);
    }

    // ---- a back-to-back ⌘⇧R must not orphan a draining recorder ----

    #[test]
    fn a_toggle_is_blocked_while_the_previous_recording_drains() {
        let (mut shell, _pty) = shell_with_terminal();
        // Idle: a toggle is free to start a recording.
        assert!(
            !shell.record_toggle_blocked(),
            "an idle shell accepts a record toggle"
        );
        // Mid-drain: a finish is pending on in-flight frame screenshots. A new
        // ⌘⇧R must be ignored, not replace the recorder under the encoder.
        shell.record_finish_pending = true;
        shell.record_inflight = 1;
        assert!(
            shell.record_toggle_blocked(),
            "a draining recorder blocks a new toggle"
        );
    }
}
