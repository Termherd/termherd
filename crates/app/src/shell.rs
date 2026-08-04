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
use termherd_core::ports::{PathResolver, ProjectScanner, PtyHost};
use termherd_core::workspace::SessionId;
use termherd_core::{
    ConfigInput, Keymap, Launch, Overlay, ScrollTarget, SelectOp, SessionRecord, SessionStatus,
};
use termherd_pty::{PtyEvent, Screen};

use crate::docs::DocEntry;
use crate::record_config::RecordConfig;
use crate::settings::{ClipboardGestures, CloseSettings, ThemeChoice};
use crate::window_config::WindowConfig;

pub(crate) mod bridge;
mod docs;
mod effects;
mod geometry;
mod ime;
mod input;
mod launch;
mod orchestrate;
mod record;
mod routing;
mod serve;
mod session_ops;
mod streams;
mod terminal;
mod view;

pub(crate) use bridge::{Requests as BridgeRequests, channel as bridge_channel};
use docs::{DocFeedback, OpenDoc};

/// The live-bridge runtime wiring handed to the shell, grouped so the
/// constructor stays within argument bounds. Carries the subscription source
/// that drains transport requests into the shell, plus the in-process MCP
/// server's endpoint and its per-session token registry — endpoint `None` and
/// tokens empty when the server did not bind.
#[derive(Clone)]
pub(crate) struct LiveBridge {
    pub(crate) requests: BridgeRequests,
    pub(crate) mcp_endpoint: Option<crate::mcp::Endpoint>,
    pub(crate) mcp_tokens: crate::mcp::Tokens,
}
use input::event_modifiers;
use record::RecordState;
use streams::{PtyOutput, pty_stream, watch_stream};

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
    /// Which mouse gestures reach the clipboard.
    pub gestures: ClipboardGestures,
    /// The editor command from settings, or `None` for the OS default handler.
    pub open: Option<termherd_core::OpenCommand>,
    /// Adapter-owned config bits for the MCP `snapshot` tool's config section
    /// (the live font size is stamped by `core`, not carried here).
    pub config: ConfigInput,
}

impl Startup {
    /// Bundle the sanitised settings with the other persisted state, so the
    /// composition root passes one value instead of fanning fields by hand.
    #[must_use]
    pub fn from_settings(
        settings: &crate::settings::Settings,
        metadata: Overlay,
        collapsed: HashSet<String>,
    ) -> Self {
        let record = settings.record_config();
        Self {
            theme: settings.theme,
            keymap: settings.keymap(),
            metadata,
            collapsed,
            session_limit: settings.session_limit(),
            font_size: settings.font_size(),
            close: settings.close,
            gestures: settings.clipboard_gestures(),
            open: settings.open_command(),
            config: ConfigInput {
                terminal_scheme: settings.terminal.colors.scheme.clone(),
                record_fps: record.fps,
                record_scale: record.scale,
                keymap_overrides: settings.keys.len(),
            },
            record,
        }
    }
}

pub fn run(
    scanner: Arc<dyn ProjectScanner>,
    watch_root: Option<PathBuf>,
    path_resolver: Arc<dyn PathResolver>,
    pty: Arc<dyn PtyHost>,
    pty_rx: UnboundedReceiver<PtyEvent>,
    live_bridge: LiveBridge,
    startup: Startup,
) -> iced::Result {
    // Restore the saved bounds, but discard a position that now lands off every
    // connected monitor (e.g. a second screen that has since been unplugged), so
    // the window can't open out of reach.
    let config =
        WindowConfig::load().with_onscreen_position(&crate::window_geometry::current_screens());
    let position = match (config.x, config.y) {
        (Some(x), Some(y)) => window::Position::Specific(Point::new(x, y)),
        _ => window::Position::Centered,
    };
    let pty_output = PtyOutput::new(pty_rx);
    iced::application(
        move || {
            let mut shell = Shell::new(
                config,
                Ports {
                    scanner: scanner.clone(),
                    watch_root: watch_root.clone(),
                    path_resolver: path_resolver.clone(),
                    pty: pty.clone(),
                    pty_output: pty_output.clone(),
                },
                live_bridge.clone(),
                Startup {
                    theme: startup.theme,
                    keymap: startup.keymap.clone(),
                    metadata: startup.metadata.clone(),
                    collapsed: startup.collapsed.clone(),
                    record: startup.record,
                    session_limit: startup.session_limit,
                    font_size: startup.font_size,
                    close: startup.close,
                    gestures: startup.gestures,
                    open: startup.open.clone(),
                    config: startup.config.clone(),
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

struct Shell {
    /// The headless core; all browser and session state lives there.
    core: termherd_core::App,
    bounds: WindowConfig,
    scanner: Arc<dyn ProjectScanner>,
    watch_root: Option<PathBuf>,
    scan_error: Option<String>,
    /// Checks whether a path-shaped run of terminal text names a real file.
    /// The one thing that tells `src/main.rs` from `and/or`.
    path_resolver: Arc<dyn PathResolver>,
    /// The PTY host adapter; effects from `core` are performed against it.
    pty: Arc<dyn PtyHost>,
    /// Streams PTY output/exit into the subscription (taken once).
    pty_output: PtyOutput,
    /// Drains async-bridge transport requests into the subscription (taken
    /// once), so an off-thread caller can read `core` state and get a reply.
    bridge_requests: BridgeRequests,
    /// The loopback MCP server's endpoint, if it bound. A Claude launch injects
    /// this url (plus a fresh token) into its `mcpServers` config. `None` when
    /// the substrate runtime or the listener failed — the browser still runs.
    mcp_endpoint: Option<crate::mcp::Endpoint>,
    /// The MCP token registry shared with the loopback server: mint one per
    /// Claude launch, revoke it when the session closes.
    mcp_tokens: crate::mcp::Tokens,
    /// The token minted for each live Claude session, so its `mcp_tokens` entry
    /// can be revoked when the session ends.
    mcp_session_tokens: HashMap<SessionId, String>,
    /// Adapter-owned config bits for the MCP `snapshot` tool's config section —
    /// the settings the pure `core` cannot read (scheme, record budget, keymap);
    /// the live font size is stamped by `core` at snapshot time.
    config: ConfigInput,
    /// Latest rendered grid per session.
    screens: HashMap<SessionId, Screen>,
    /// Bridge callers parked on a session reaching a target activity — the one
    /// request kind whose reply lands in a later `update` (see `shell::serve`).
    waiters: Vec<serve::StatusWaiter>,
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
    /// Which mouse gestures reach the clipboard (from `settings.json`), handed
    /// to each terminal canvas so a drag release or a right-click knows whether
    /// it is a clipboard gesture at all.
    gestures: ClipboardGestures,
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
    /// Whether Shift is currently held, handed to the terminal canvas so a
    /// Shift+click extends the existing selection instead of restarting it.
    shift_modifier: bool,
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
    /// The GIF screencast (F-capture rung 2): the recording budget and the
    /// in-progress encoder state, encapsulated so the shell holds one field.
    record: RecordState,
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
    /// A session's shell announced the directory it moved to (OSC 7).
    PtyCwd {
        session: SessionId,
        cwd: String,
    },
    /// A session fired an OSC 9 notification; forward it to the OS.
    PtyNotify {
        session: SessionId,
        body: String,
    },
    /// A session's process exited; `clean` mirrors [`PtyEvent::Exited`].
    PtyExited {
        session: SessionId,
        clean: bool,
    },
    /// A raw key press; routed to the focused terminal when it has focus.
    Key(keyboard::Event),
    /// IME-composed text (dead/accent keys, CJK) for the focused terminal.
    ImeCommit(String),
    /// Give keyboard focus to the search box.
    FocusSearch,
    /// Click-to-focus the pane hosting a session (FR6): moves pane focus there
    /// and gives the keyboard to the terminal. A lone terminal is the one-leaf
    /// case, focused the same way.
    FocusPane(SessionId),
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
    /// Change a terminal's grid-anchored selection — press, drag, or clear — so
    /// the highlight follows the text through scroll (FR4).
    Select {
        session: SessionId,
        op: SelectOp,
    },
    /// Set a terminal's selection (a double-click word) and copy it at once, so
    /// the highlight persists and tracks while the word lands on the clipboard.
    SelectAndCopy {
        session: SessionId,
        op: SelectOp,
        text: String,
    },
    /// Ask a terminal to copy its current selection (a drag release). The text
    /// is read from the live grid selection and returned out-of-band, so it is
    /// exact even right after a fast drag whose highlight has not echoed back.
    RequestCopySelection {
        session: SessionId,
    },
    /// Copy the given text (a terminal selection) to the clipboard (FR4).
    CopySelection(String),
    /// Clipboard contents read back for a paste into the focused terminal (FR4).
    Paste(Option<String>),
    /// A right-click asked to paste into the pane under the pointer, which is
    /// not necessarily the focused one. Reads the clipboard, then
    /// [`Message::PasteInto`] lands it.
    RequestPaste {
        session: SessionId,
    },
    /// Clipboard contents read back for a paste into a named session.
    PasteInto {
        session: SessionId,
        content: Option<String>,
    },
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
    /// Abandon the inline rename (Escape), keeping the previous title — the
    /// same outcome a blur gives, which is what the sidebar's field does.
    CancelRename,
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
    /// The clickable target now under the pointer in a terminal, or `None` when
    /// the pointer left every one. The canvas finds it; `core` owns it.
    TermTarget {
        session: termherd_core::SessionId,
        probe: Option<termherd_core::TargetProbe>,
    },
    /// The user Ctrl/Cmd+clicked a terminal target — a URL or a file path.
    ActivateTarget {
        session: termherd_core::SessionId,
        probe: termherd_core::TargetProbe,
    },
    /// A path candidate came back from the resolver: the file it names, or
    /// `None` when it names none.
    PathResolved {
        request: termherd_core::PathRequest,
        resolved: Option<termherd_core::ResolvedPath>,
    },
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
    /// A transport task asked the running app something over the async bridge:
    /// read the answer from `core` and send it back on `reply`. Every such call
    /// is timeout-bounded on the caller's side, so a slow answer degrades that
    /// one request, never the shell.
    Bridge {
        request: bridge::Request,
        reply: bridge::ReplyPort,
    },
}

impl Message {
    /// Whether this message is a deliberate user interaction *elsewhere* in the
    /// UI that should cancel an in-progress inline rename. This is an explicit
    /// allowlist, not a blocklist: anything unlisted (PTY output, scans, window
    /// and key events, and the rename's own `StartRename`/`RenameInput`/
    /// `CommitRename`/`CancelRename`) leaves the edit untouched. Defaulting to
    /// "don't dismiss" is the safe side — a missed button is a minor gap,
    /// whereas a stray background message dismissing the edit would make
    /// renaming impossible.
    fn dismisses_rename(&self) -> bool {
        matches!(
            self,
            Self::SearchChanged(_)
                | Self::SearchTitlesOnly(_)
                | Self::LaunchProject(_)
                | Self::LaunchSession { .. }
                | Self::FocusSearch
                | Self::FocusPane(_)
                | Self::TermScroll { .. }
                | Self::Paste(_)
                | Self::RequestPaste { .. }
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

/// Everything the shell reaches the outside world through, constructed in
/// `main()` and injected as one piece. Grouped because they travel together and
/// are chosen together: swapping the real adapters for test doubles is one
/// substitution, not five.
pub(crate) struct Ports {
    pub(crate) scanner: Arc<dyn ProjectScanner>,
    /// The projects tree to watch for changes, when there is one.
    pub(crate) watch_root: Option<PathBuf>,
    pub(crate) path_resolver: Arc<dyn PathResolver>,
    pub(crate) pty: Arc<dyn PtyHost>,
    pub(crate) pty_output: PtyOutput,
}

impl Shell {
    fn new(bounds: WindowConfig, ports: Ports, live_bridge: LiveBridge, startup: Startup) -> Self {
        let Ports {
            scanner,
            watch_root,
            path_resolver,
            pty,
            pty_output,
        } = ports;
        let LiveBridge {
            requests: bridge_requests,
            mcp_endpoint,
            mcp_tokens,
        } = live_bridge;
        let mut core = termherd_core::App::new();
        core.apply(termherd_core::Event::MetadataLoaded(startup.metadata));
        core.apply(termherd_core::Event::CollapsedLoaded(startup.collapsed));
        core.apply(termherd_core::Event::SessionLimitLoaded(
            startup.session_limit,
        ));
        core.apply(termherd_core::Event::FontSizeLoaded(startup.font_size));
        core.apply(termherd_core::Event::OpenCommandLoaded(startup.open));
        Self {
            core,
            bounds,
            scanner,
            watch_root,
            scan_error: None,
            path_resolver,
            pty,
            pty_output,
            bridge_requests,
            mcp_endpoint,
            mcp_tokens,
            mcp_session_tokens: HashMap::new(),
            screens: HashMap::new(),
            waiters: Vec::new(),
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
            gestures: startup.gestures,
            archiving: None,
            closing_window: None,
            link_modifier: false,
            shift_modifier: false,
            tab_drag: None,
            exiting: false,
            record: RecordState::new(startup.record),
            config: startup.config,
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
        let paths: Vec<String> = self
            .core
            .sidebar
            .projects
            .iter()
            .map(|g| g.path.clone())
            .collect();
        Task::perform(
            async move { crate::docs::discover(&paths) },
            Message::DocsDiscovered,
        )
    }

    // The iced `update` is a flat `match` over every `Message` variant — the
    // app's central event dispatcher, whose arms delegate to the domain modules
    // (launch, session_ops, effects, geometry, routing, …). The length here is
    // breadth (one arm per message), not nested complexity, and splitting the
    // dispatch itself would only scatter it — the allow is by design, not a
    // deferred cleanup.
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
            let active = self.core.workspace.tab_of(anchor).unwrap_or(usize::MAX);
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
                let effects = self.core.apply(termherd_core::Event::SearchChanged(query));
                self.perform(effects)
            }
            Message::SearchTitlesOnly(titles_only) => {
                let effects = self
                    .core
                    .apply(termherd_core::Event::SearchTitlesOnlyToggled(titles_only));
                self.perform(effects)
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
                let effects = self
                    .core
                    .apply(termherd_core::Event::StatusChanged { session, status });
                self.settle_waiters(session);
                self.perform(effects)
            }
            Message::PtyTitle { session, title } => {
                let effects = self
                    .core
                    .apply(termherd_core::Event::SessionTitleChanged { session, title });
                self.perform(effects)
            }
            Message::PtyCwd { session, cwd } => {
                let effects = self
                    .core
                    .apply(termherd_core::Event::SessionCwdChanged { session, cwd });
                self.perform(effects)
            }
            Message::PtyNotify { session, body } => {
                // Unlike status/title, this yields an `Effect::Notify` that the
                // shell must perform — hand it to the OS notification centre.
                let effects = self
                    .core
                    .apply(termherd_core::Event::SessionNotified { session, body });
                self.perform(effects)
            }
            Message::PtyExited { session, clean } => {
                let tabs_before = self.core.workspace.tabs.len();
                let effects = self
                    .core
                    .apply(termherd_core::Event::PtyExited { session, clean });
                // An exit is the other way a session's activity settles: a crash
                // records `Exited` without emitting a status change, and a clean
                // exit takes the session out of the registry entirely.
                self.settle_waiters(session);
                if effects.is_empty() {
                    // No auto-close: the dead terminal stays on screen.
                    Task::none()
                } else {
                    // The pane auto-closed on its clean shell exit — mirror
                    // `close_tab`'s shell-side hygiene for the vanished session.
                    self.screens.remove(&session);
                    if self.core.workspace.tabs.len() != tabs_before {
                        // Tab indices shifted under any pending close
                        // confirmation; dropping the prompt is the safe
                        // reaction (the user can re-request).
                        self.closing = None;
                    }
                    Task::batch([self.perform(effects), self.resize_panes()])
                }
            }
            Message::Key(event) => {
                // Keep the link-open modifier state current regardless of focus,
                // so a Ctrl/Cmd+hover highlights links even before the first key
                // reaches the terminal.
                let modifiers = event_modifiers(&event);
                self.link_modifier = modifiers.control() || modifiers.logo();
                self.shift_modifier = modifiers.shift();
                // A real keypress has no one to report to; the verdict exists
                // for the MCP press tool, which answers a caller.
                self.on_key(event).1
            }
            Message::ImeCommit(text) => self.on_ime_commit(text),
            Message::FocusPane(session) => self.focus_pane(session),
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
            Message::Select { session, op } => {
                let effects = self
                    .core
                    .apply(termherd_core::Event::Select { session, op });
                self.perform(effects)
            }
            Message::SelectAndCopy { session, op, text } => {
                let effects = self
                    .core
                    .apply(termherd_core::Event::Select { session, op });
                let select = self.perform(effects);
                if text.is_empty() {
                    select
                } else {
                    self.selection = Some(text.clone());
                    Task::batch([select, iced::clipboard::write(text)])
                }
            }
            Message::RequestCopySelection { session } => {
                let effects = self
                    .core
                    .apply(termherd_core::Event::CopyTerminalSelection { session });
                self.perform(effects)
            }
            Message::CopySelection(text) => {
                if text.is_empty() {
                    Task::none()
                } else {
                    self.selection = Some(text.clone());
                    iced::clipboard::write(text)
                }
            }
            Message::Paste(content) => {
                let Some(session) = self.core.workspace.focused_session() else {
                    return Task::none();
                };
                self.paste_into(session, content)
            }
            Message::RequestPaste { session } => {
                // A prompt that owns the keyboard owns the input. The pointer
                // must not be a way past a confirmation the keyboard cannot
                // pass — the paste chord is swallowed there, and so is this.
                if self.keyboard_owner().is_some() {
                    return Task::none();
                }
                // The pane you paste into is the pane you are now working in;
                // every terminal focuses the pane its paste-click landed on,
                // and the left button already focuses through `mouse_area`.
                let focus = self.focus_pane(session);
                let read = iced::clipboard::read()
                    .map(move |content| Message::PasteInto { session, content });
                Task::batch([focus, read])
            }
            Message::PasteInto { session, content } => self.paste_into(session, content),
            Message::RequestCloseTab(index) => self.request_close(index).unwrap_or_else(Task::none),
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
                    let effects = self
                        .core
                        .apply(termherd_core::Event::MoveTab { from, to: over });
                    self.perform(effects)
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
                Some(session) if self.core.is_browsable(&session) => {
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
                let effects = self
                    .core
                    .apply(termherd_core::Event::ShowArchivedToggled(show));
                self.perform(effects)
            }
            Message::ToggleCollapsed(path) => {
                let effects = self.core.apply(termherd_core::Event::ToggleCollapsed(path));
                self.perform(effects)
            }
            Message::ToggleExpanded(path) => {
                let effects = self.core.apply(termherd_core::Event::ToggleExpanded(path));
                self.perform(effects)
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
            Message::CancelRename => {
                self.renaming = None;
                Task::none()
            }
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
            Message::TermTarget { session, probe } => {
                let effects = self
                    .core
                    .apply(termherd_core::Event::TermTarget { session, probe });
                self.perform(effects)
            }
            Message::ActivateTarget { session, probe } => {
                let effects = self
                    .core
                    .apply(termherd_core::Event::ActivateTarget { session, probe });
                self.perform(effects)
            }
            Message::PathResolved { request, resolved } => {
                let effects = self
                    .core
                    .apply(termherd_core::Event::PathResolved { request, resolved });
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
            Message::RecordFrame(screenshot) => self.record.on_frame(screenshot),
            Message::Bridge { request, reply } => self.serve(request, reply),
        }
    }

    /// Put the terminal selection on the clipboard (FR4). `None` when there is
    /// nothing selected: a caller told the copy ran would follow with a paste and
    /// paste whatever was on the clipboard before.
    ///
    /// A **visible** selection outranks [`Self::selection`], which is a cache of
    /// the last text copied, not of the last text selected. With copy-on-select
    /// off, nothing but a copy fills that cache — so trusting it first would put
    /// the previously copied text on the clipboard while a fresh highlight sits
    /// on screen, silently copying the wrong thing. The terminal reads its own
    /// live selection and answers out of band, which refills the cache on the
    /// way. The cache still serves the case the screen cannot: a selection
    /// scrolled out of the viewport carries no spans to see.
    fn copy_selection(&mut self) -> Option<Task<Message>> {
        if let Some(session) = self.core.workspace.focused_session()
            && self
                .screens
                .get(&session)
                .is_some_and(|screen| !screen.selection.is_empty())
        {
            let effects = self
                .core
                .apply(termherd_core::Event::CopyTerminalSelection { session });
            return Some(self.perform(effects));
        }
        match &self.selection {
            Some(sel) if !sel.is_empty() => Some(iced::clipboard::write(sel.clone())),
            _ => None,
        }
    }

    /// Move pane focus to `session` and give the keyboard to its terminal —
    /// what a click on a pane means, whichever button made it.
    fn focus_pane(&mut self, session: SessionId) -> Task<Message> {
        self.focus = Focus::Terminal;
        let effects = self.core.apply(termherd_core::Event::FocusPane(session));
        self.perform(effects)
    }

    /// Write clipboard `content` into `session` as terminal input, bracketed
    /// when that session asked for it. The one paste seam: the keyboard chord
    /// arrives here with the focused session, a right-click with the one under
    /// the pointer, so neither can grow its own idea of how a paste is framed.
    /// Empty (or absent) content writes nothing — a paste of nothing must not
    /// send the bracket markers on their own.
    fn paste_into(&mut self, session: SessionId, content: Option<String>) -> Task<Message> {
        let Some(text) = content.filter(|t| !t.is_empty()) else {
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
        let Some(index) = self.core.workspace.tab_of(anchor) else {
            return;
        };
        let effects = self
            .core
            .apply(termherd_core::Event::RenameTab { index, title });
        debug_assert!(effects.is_empty());
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
        subs.push(Subscription::run_with(
            self.bridge_requests.clone(),
            bridge::request_stream,
        ));
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
    use super::routing::KeyboardOwner;
    use super::*;
    use crate::settings::ConfirmClose;
    use iced::keyboard::key::{Named, NativeCode, Physical};
    use iced::keyboard::{Key, Location, Modifiers};
    use std::sync::Mutex as StdMutex;
    use termherd_core::ports::{PtyError, ScanError};
    use termherd_core::{Action, SelectSide, SnapshotFilter, SpawnSpec};

    /// A `PtyHost` double recording every write and kill; all calls succeed.
    #[derive(Default)]
    struct RecordingPty {
        writes: StdMutex<Vec<(SessionId, Vec<u8>)>>,
        kills: StdMutex<usize>,
        spawns: StdMutex<usize>,
        launches: StdMutex<Vec<Launch>>,
        resizes: StdMutex<Vec<(u16, u16)>>,
        scrolls: StdMutex<Vec<ScrollTarget>>,
        selects: StdMutex<Vec<SelectOp>>,
        copies: StdMutex<usize>,
    }

    impl RecordingPty {
        fn writes(&self) -> Vec<Vec<u8>> {
            self.writes_seen().into_iter().map(|(_, b)| b).collect()
        }
        /// Every write with the session it landed in — what a test asserting
        /// *which pane* received the bytes needs.
        fn writes_seen(&self) -> Vec<(SessionId, Vec<u8>)> {
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
        fn selects(&self) -> Vec<SelectOp> {
            self.selects.lock().expect("selects lock").clone()
        }
        fn copy_count(&self) -> usize {
            *self.copies.lock().expect("copies lock")
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
        fn write(&self, session: SessionId, bytes: &[u8]) -> Result<(), PtyError> {
            self.writes
                .lock()
                .expect("writes lock")
                .push((session, bytes.to_vec()));
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
        fn select(&self, _: SessionId, op: SelectOp) -> Result<(), PtyError> {
            self.selects.lock().expect("selects lock").push(op);
            Ok(())
        }
        fn copy_selection(&self, _: SessionId) -> Result<(), PtyError> {
            *self.copies.lock().expect("copies lock") += 1;
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

    /// The default startup payload the test shells boot with.
    fn test_startup() -> Startup {
        Startup {
            theme: ThemeChoice::default(),
            keymap: Keymap::defaults(),
            metadata: Overlay::default(),
            collapsed: HashSet::new(),
            record: RecordConfig::default(),
            session_limit: 0,
            font_size: 14.0,
            close: CloseSettings::default(),
            gestures: ClipboardGestures::default(),
            open: None,
            config: test_config_input(),
        }
    }

    /// A neutral config input for test shells — the snapshot tool's config
    /// section is exercised in the `core` builder tests, not here.
    fn test_config_input() -> ConfigInput {
        ConfigInput {
            terminal_scheme: None,
            record_fps: 8,
            record_scale: 0.5,
            keymap_overrides: 0,
        }
    }

    /// Live-bridge wiring for a test shell: a fresh request channel and no MCP
    /// server (endpoint `None`), so tests that don't exercise the bridge are
    /// unaffected. Tests that do set `shell.mcp_endpoint` afterwards.
    fn test_live_bridge() -> LiveBridge {
        LiveBridge {
            requests: bridge::channel().1,
            mcp_endpoint: None,
            mcp_tokens: crate::mcp::Tokens::default(),
        }
    }

    /// The real scan-side adapters over a test PTY: an empty scanner, no watch
    /// root, and the genuine path resolver — it only ever sees paths a test
    /// wrote to a tempdir, so a double would test less.
    fn test_ports(
        pty: Arc<dyn PtyHost>,
        rx: iced::futures::channel::mpsc::UnboundedReceiver<PtyEvent>,
    ) -> Ports {
        Ports {
            scanner: Arc::new(EmptyScanner),
            watch_root: None,
            path_resolver: Arc::new(termherd_scan::FsPathResolver::new()),
            pty,
            pty_output: PtyOutput::new(rx),
        }
    }

    /// A `Shell` over the given PTY host, with no terminal open yet.
    fn shell_over(pty: Arc<dyn PtyHost>) -> Shell {
        let (_tx, rx) = iced::futures::channel::mpsc::unbounded::<PtyEvent>();
        Shell::new(
            WindowConfig::default(),
            test_ports(pty, rx),
            test_live_bridge(),
            test_startup(),
        )
    }

    #[test]
    fn a_configured_open_command_reaches_core_from_settings_json() {
        // The one seam neither crate's own tests cover: `settings.json` →
        // `Startup` → `Event::OpenCommandLoaded`. A command that parses in
        // isolation but never reaches `core` would leave the click opening
        // through the OS, and every unit test would still pass.
        let settings: crate::settings::Settings =
            serde_json::from_str(r#"{ "open": { "command": "code -g {path}:{line}:{col}" } }"#)
                .expect("valid json");
        let (_tx, rx) = iced::futures::channel::mpsc::unbounded::<PtyEvent>();
        let mut shell = Shell::new(
            WindowConfig::default(),
            test_ports(Arc::new(RecordingPty::default()), rx),
            test_live_bridge(),
            Startup::from_settings(&settings, Overlay::default(), HashSet::new()),
        );

        let request = termherd_core::PathRequest {
            session: termherd_core::SessionId(std::num::NonZeroU64::MIN),
            purpose: termherd_core::PathPurpose::Open,
            row: 3,
            start: 0,
            end: 11,
            candidate: "src/main.rs".to_owned(),
            line: Some(42),
            col: None,
        };
        let effects = shell.core.apply(termherd_core::Event::PathResolved {
            request,
            resolved: Some(termherd_core::ResolvedPath {
                path: "/repo/src/main.rs".into(),
                real: "/repo/src/main.rs".into(),
            }),
        });
        match effects.as_slice() {
            [
                termherd_core::Effect::OpenPath(termherd_core::OpenTarget::Editor {
                    program,
                    args,
                }),
            ] => {
                assert_eq!(program, "code");
                assert_eq!(args, &["-g", "/repo/src/main.rs:42:1"]);
            }
            other => panic!("expected the configured editor, got {other:?}"),
        }
    }

    /// A `Shell` with one terminal open and focused, plus its recording PTY.
    fn shell_with_terminal() -> (Shell, Arc<RecordingPty>) {
        let pty = Arc::new(RecordingPty::default());
        let mut shell = shell_over(pty.clone());
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
    fn snapshot_inputs_gather_config_when_asked_and_scope_terminal_text() {
        use super::bridge::Request;
        use termherd_core::{Section, SnapshotFilter, TerminalScope};

        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        shell.screens.insert(session, screen_of("$ cargo test"));
        let handle = session.0.get();

        // Config section on, and the focused handle's terminal text requested.
        let inputs = shell.snapshot_inputs(&Request::Snapshot(SnapshotFilter {
            sections: vec![Section::Config],
            terminals: TerminalScope::Only(vec![handle]),
            text_lines: 40,
        }));
        assert!(inputs.config.is_some(), "the config section was requested");
        assert!(
            inputs
                .terminals
                .get(&handle)
                .is_some_and(|text| text.contains("cargo test")),
            "the scoped terminal's text is gathered from its screen"
        );

        // Config section off and no terminal scope: nothing gathered.
        let bare = shell.snapshot_inputs(&Request::Snapshot(SnapshotFilter {
            sections: vec![Section::Tabs],
            terminals: TerminalScope::None,
            text_lines: 40,
        }));
        assert!(bare.config.is_none(), "config off → not gathered");
        assert!(bare.terminals.is_empty(), "no terminal scope → no text");
    }

    // ---- Orchestration actions (F-mcp-orchestration) --------------------
    //
    // Each MCP action is a thin wrapper over an existing core event. These
    // pin the shell-side seam: handle resolution, the applied mutation, and
    // the reported resulting focus.

    // `Action`/`SessionKind` already name the `termherd_core` types in this
    // module, so the bridge's carry a `Bridge` prefix here.
    use super::bridge::{Action as BridgeAction, SessionKind as BridgeKind};
    use termherd_core::workspace::SplitDir;

    /// The focused session's handle string, or `None` when nothing is focused.
    fn focused(shell: &Shell) -> Option<String> {
        shell
            .core
            .workspace
            .focused_session()
            .map(|id| id.0.get().to_string())
    }

    #[test]
    fn open_action_launches_and_focuses_a_new_pane() {
        let pty = Arc::new(RecordingPty::default());
        let mut shell = shell_over(pty.clone());
        let (outcome, _task) = shell.perform_action(BridgeAction::Open {
            project: Some("/tmp/x".into()),
            kind: BridgeKind::Shell,
        });
        assert_eq!(outcome.error, None, "opening a session never rejects");
        assert_eq!(
            outcome.focused,
            focused(&shell),
            "the outcome reports the newly focused pane"
        );
        assert!(outcome.focused.is_some(), "the new pane holds focus");
        assert_eq!(pty.spawn_count(), 1, "one PTY was spawned");
        assert_eq!(pty.launches(), vec![Launch::Shell]);
    }

    #[test]
    fn open_action_defaults_to_the_home_dir_and_kind_claude() {
        let pty = Arc::new(RecordingPty::default());
        let mut shell = shell_over(pty.clone());
        let (outcome, _task) = shell.perform_action(BridgeAction::Open {
            project: None,
            kind: BridgeKind::Claude,
        });
        assert_eq!(outcome.error, None);
        assert_eq!(
            pty.launches(),
            vec![Launch::Claude { resume: None }],
            "a fresh Claude session, no project → home dir"
        );
    }

    #[test]
    fn run_action_writes_the_bytes_to_the_target_pty() {
        let (mut shell, pty) = shell_with_terminal();
        let handle = focused(&shell).expect("a focused session");
        let (outcome, _task) = shell.perform_action(BridgeAction::Run {
            session: handle.parse().expect("numeric handle"),
            bytes: b"ls\n".to_vec(),
        });
        assert_eq!(outcome.error, None);
        assert_eq!(pty.writes(), vec![b"ls\n".to_vec()], "the bytes were typed");
    }

    #[test]
    fn run_action_rejects_an_unknown_handle_without_writing() {
        let (mut shell, pty) = shell_with_terminal();
        let (outcome, _task) = shell.perform_action(BridgeAction::Run {
            session: 999,
            bytes: b"rm -rf /\n".to_vec(),
        });
        assert!(
            outcome.error.is_some_and(|e| e.contains("999")),
            "an unknown handle is rejected, naming it"
        );
        assert!(pty.writes().is_empty(), "nothing was typed into any PTY");
    }

    #[test]
    fn focus_action_rejects_an_unknown_handle() {
        let (mut shell, _pty) = shell_with_terminal();
        let before = focused(&shell);
        let (outcome, _task) = shell.perform_action(BridgeAction::Focus { session: 999 });
        assert!(outcome.error.is_some(), "an unknown handle is rejected");
        assert_eq!(focused(&shell), before, "focus is untouched");
    }

    #[test]
    fn focus_action_moves_focus_to_the_target_pane_in_a_split() {
        let (mut shell, _pty) = shell_with_terminal();
        let first = focused(&shell).expect("first pane");
        // Split so the tab holds two panes; the new one takes focus.
        let (split, _task) = shell.perform_action(BridgeAction::Split {
            pane: None,
            dir: SplitDir::Vertical,
        });
        assert_eq!(split.error, None);
        assert_ne!(
            focused(&shell),
            Some(first.clone()),
            "the new pane is focused"
        );
        // Focus back to the first pane by its handle.
        let (outcome, _task) = shell.perform_action(BridgeAction::Focus {
            session: first.parse().expect("numeric handle"),
        });
        assert_eq!(outcome.error, None);
        assert_eq!(focused(&shell), Some(first), "focus returned to the target");
    }

    #[test]
    fn split_action_opens_and_focuses_a_pane_beside_the_focused_one() {
        let (mut shell, pty) = shell_with_terminal();
        let first = focused(&shell).expect("first pane");
        let spawns_before = pty.spawn_count();
        let (outcome, _task) = shell.perform_action(BridgeAction::Split {
            pane: None,
            dir: SplitDir::Horizontal,
        });
        assert_eq!(outcome.error, None);
        let active = shell.core.workspace.active;
        assert_eq!(
            shell.core.workspace.tabs[active].sessions().len(),
            2,
            "the tab now hosts two panes"
        );
        assert_eq!(
            outcome.focused,
            focused(&shell),
            "the outcome reports the new focus"
        );
        assert_ne!(outcome.focused, Some(first), "the fresh pane holds focus");
        assert_eq!(pty.spawn_count(), spawns_before + 1, "one more PTY spawned");
    }

    #[test]
    fn split_action_rejects_an_unknown_target_pane() {
        let (mut shell, _pty) = shell_with_terminal();
        let (outcome, _task) = shell.perform_action(BridgeAction::Split {
            pane: Some(999),
            dir: SplitDir::Vertical,
        });
        assert!(
            outcome.error.is_some(),
            "an unknown target pane is rejected"
        );
        assert_eq!(
            shell.core.workspace.tabs[shell.core.workspace.active]
                .sessions()
                .len(),
            1,
            "no split happened"
        );
    }

    #[test]
    fn rename_action_relabels_the_tab() {
        let (mut shell, _pty) = shell_with_terminal();
        let (outcome, _task) = shell.perform_action(BridgeAction::Rename {
            tab: 0,
            title: "build".into(),
        });
        assert_eq!(outcome.error, None);
        assert_eq!(shell.core.workspace.tabs[0].display_title(), "build");
    }

    #[test]
    fn rename_action_rejects_an_out_of_range_tab() {
        let (mut shell, _pty) = shell_with_terminal();
        let (outcome, _task) = shell.perform_action(BridgeAction::Rename {
            tab: 9,
            title: "nope".into(),
        });
        assert!(outcome.error.is_some(), "a missing tab is rejected");
    }

    #[test]
    fn close_action_closes_a_lone_pane_tab_and_kills_its_pty() {
        let (mut shell, pty) = shell_with_terminal();
        assert_eq!(shell.core.workspace.tabs.len(), 1);
        let (outcome, _task) = shell.perform_action(BridgeAction::Close { pane: None });
        assert_eq!(outcome.error, None);
        assert!(
            shell.core.workspace.tabs.is_empty(),
            "the lone tab is closed"
        );
        assert_eq!(pty.kill_count(), 1, "its PTY was killed");
    }

    /// A shell with two tabs; returns it plus the session handle of the *first*
    /// tab, which is no longer the active one — the setup for the cross-tab
    /// targeting tests below.
    fn shell_with_two_tabs() -> (Shell, Arc<RecordingPty>, u64) {
        let (mut shell, pty) = shell_with_terminal();
        let first = shell
            .core
            .workspace
            .focused_session()
            .expect("focused")
            .0
            .get();
        let _ = shell.launch("/tmp/other".to_string(), Launch::Shell);
        assert_eq!(shell.core.workspace.active, 1, "the new tab is active");
        (shell, pty, first)
    }

    #[test]
    fn close_action_reaches_a_pane_in_another_tab() {
        // The registry is workspace-global, so a handle from an inactive tab
        // resolves; the close must land on *that* pane, not on the active tab's
        // focused one.
        let (mut shell, _pty, first) = shell_with_two_tabs();
        let (outcome, _task) = shell.perform_action(BridgeAction::Close { pane: Some(first) });
        assert_eq!(outcome.error, None);
        let live: Vec<u64> = shell
            .core
            .workspace
            .tabs
            .iter()
            .flat_map(|tab| tab.sessions())
            .map(|id| id.0.get())
            .collect();
        assert!(
            !live.contains(&first),
            "the targeted pane is gone; surviving panes: {live:?}"
        );
        assert_eq!(live.len(), 1, "the other tab must not have been closed");
    }

    #[test]
    fn split_action_reaches_a_pane_in_another_tab() {
        let (mut shell, _pty, first) = shell_with_two_tabs();
        let (outcome, _task) = shell.perform_action(BridgeAction::Split {
            pane: Some(first),
            dir: SplitDir::Vertical,
        });
        assert_eq!(outcome.error, None);
        assert_eq!(
            shell.core.workspace.active, 0,
            "the split happened in the target's tab"
        );
        assert_eq!(
            shell.core.workspace.tabs[0].sessions().len(),
            2,
            "the target tab now hosts two panes"
        );
        assert_eq!(shell.core.workspace.tabs[1].sessions().len(), 1);
    }

    #[test]
    fn focus_action_reaches_a_pane_in_another_tab() {
        let (mut shell, _pty, first) = shell_with_two_tabs();
        let (outcome, _task) = shell.perform_action(BridgeAction::Focus { session: first });
        assert_eq!(outcome.error, None);
        assert_eq!(shell.core.workspace.active, 0, "its tab was activated");
        assert_eq!(
            outcome.focused,
            Some(first.to_string()),
            "the reported focus is the pane that was asked for"
        );
        assert_eq!(focused(&shell), Some(first.to_string()));
    }

    #[test]
    fn focus_relative_actions_reject_a_handle_whose_pane_is_gone() {
        // A stale handle — the agent read it, then the pane closed. Rejecting
        // beats silently acting on whatever holds focus now, which is how a
        // close request would destroy the wrong terminal.
        let (mut shell, pty, first) = shell_with_two_tabs();
        let (outcome, _task) = shell.perform_action(BridgeAction::Close { pane: Some(first) });
        assert_eq!(outcome.error, None, "the first close lands");
        let handle = first;

        for action in [
            BridgeAction::Close { pane: Some(handle) },
            BridgeAction::Split {
                pane: Some(handle),
                dir: SplitDir::Vertical,
            },
            BridgeAction::Focus { session: handle },
        ] {
            let (outcome, _task) = shell.perform_action(action.clone());
            assert_eq!(
                outcome.error,
                Some(format!("no open pane hosts handle {handle}")),
                "{action:?} should be rejected"
            );
        }
        assert_eq!(
            shell.core.workspace.tabs.len(),
            1,
            "the surviving tab is untouched"
        );
        assert_eq!(pty.kill_count(), 1, "only the first, targeted close killed");
    }

    // ---- Key presses over the control surface (F-mcp-keys) ---------------
    //
    // Two tools share one dispatch: a chord walks the real `on_key` ladder as a
    // synthesised event, a named action skips the keymap but not the ladder.
    // These pin what the ladder *reports*, which is the half a caller reads.

    use super::bridge::{Press, PressStep};
    use termherd_core::KeyChord;

    /// Press one chord spec and return the step it produced.
    fn press_chord(shell: &mut Shell, spec: &str) -> PressStep {
        press_all(shell, &[spec]).remove(0)
    }

    /// Press a sequence of chord specs and return every step, in order.
    fn press_all(shell: &mut Shell, specs: &[&str]) -> Vec<PressStep> {
        let presses = specs
            .iter()
            .map(|spec| Press::Chord(KeyChord::parse(spec).expect("a test chord parses")))
            .collect();
        let (outcome, _task) = shell.perform_presses(presses);
        assert_eq!(outcome.error, None, "the shell routes presses itself");
        outcome.steps
    }

    /// The platform's primary-modifier spec, so these tests read the same
    /// bindings the running app does on macOS and elsewhere.
    fn mod_spec(rest: &str) -> String {
        let primary = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        format!("{primary}+{rest}")
    }

    #[test]
    fn a_bound_chord_runs_its_action_and_names_it() {
        // The binding half: the chord resolves through the *live* keymap, so
        // what runs is whatever the user bound — and the step names it, so a
        // caller can tell "it worked" from "it hit something else".
        let (mut shell, _pty) = shell_with_terminal();
        let panes_before = shell.core.workspace.tabs[0].sessions().len();
        let step = press_chord(&mut shell, &mod_spec("d"));
        assert_eq!(step, PressStep::Ran("split-vertical".to_owned()));
        assert_eq!(
            shell.core.workspace.tabs[0].sessions().len(),
            panes_before + 1,
            "the action must actually apply, not just report"
        );
    }

    #[test]
    fn an_unbound_chord_reaches_the_focused_terminal_as_text() {
        // A chord bound to nothing falls through exactly as a keypress does.
        // Reported as `Typed`, not as success or as silence — the caller needs
        // to know its chord went to the shell instead of to the app.
        let (mut shell, pty) = shell_with_terminal();
        let step = press_chord(&mut shell, "x");
        assert_eq!(step, PressStep::Typed);
        assert_eq!(
            pty.writes(),
            vec![b"x".to_vec()],
            "the character must reach the PTY"
        );
    }

    #[test]
    fn a_chord_with_no_binding_and_no_terminal_is_reported_unbound() {
        // Nothing claimed it. Reported rather than dropped: a silent success
        // here would have an agent believe a gesture it never made.
        let (mut shell, pty) = shell_with_terminal();
        shell.focus = Focus::Search;
        let step = press_chord(&mut shell, "x");
        assert_eq!(step, PressStep::Unbound);
        assert!(pty.writes().is_empty(), "nothing was typed anywhere");
    }

    #[test]
    fn a_chord_naming_an_unreachable_key_is_reported_unbound() {
        // `f2` is a key no event carries, so no real press could resolve it
        // either. It must report, not vanish.
        let (mut shell, _pty) = shell_with_terminal();
        assert_eq!(press_chord(&mut shell, "f2"), PressStep::Unbound);
    }

    #[test]
    fn a_press_sequence_applies_in_order() {
        // Each press lands before the next, so a sequence composes — two splits
        // leave three panes, not two.
        let (mut shell, _pty) = shell_with_terminal();
        let steps = press_all(&mut shell, &[&mod_spec("d"), &mod_spec("d")]);
        assert_eq!(
            steps,
            vec![
                PressStep::Ran("split-vertical".to_owned()),
                PressStep::Ran("split-vertical".to_owned()),
            ]
        );
        assert_eq!(shell.core.workspace.tabs[0].sessions().len(), 3);
    }

    #[test]
    fn a_press_reports_the_focus_it_left_behind() {
        // act→observe in one round trip, like the other action tools: a split
        // focuses the new pane, and that is the handle the caller gets back.
        let (mut shell, _pty) = shell_with_terminal();
        let presses = vec![Press::Chord(
            KeyChord::parse(&mod_spec("d")).expect("chord parses"),
        )];
        let (outcome, _task) = shell.perform_presses(presses);
        assert_eq!(outcome.focused, focused(&shell));
        assert!(outcome.focused.is_some(), "the new pane holds focus");
    }

    #[test]
    fn a_prompt_a_press_opened_consumes_the_next_press_and_says_which() {
        // The reason a chord is dispatched as a synthesised *event* rather than
        // resolved to its action: closing a busy session arms a confirmation,
        // and from then on the app behaves for MCP exactly as it does for a
        // human — the prompt eats the next chord. The step names the prompt, so
        // a caller learns why its chord did nothing instead of guessing.
        let (mut shell, pty) = busy_shell_with_terminal();
        assert_eq!(
            press_chord(&mut shell, &mod_spec("w")),
            PressStep::Ran("close-focused".to_owned())
        );
        assert!(shell.closing.is_some(), "a busy session arms the prompt");
        assert_eq!(
            press_chord(&mut shell, &mod_spec("d")),
            PressStep::Overlay("tab-close-confirm".to_owned()),
            "the prompt owns the keyboard, so the split never happens"
        );
        assert_eq!(shell.core.workspace.tabs[0].sessions().len(), 1);
        assert_eq!(pty.kill_count(), 0, "nothing was killed while parked");
    }

    #[test]
    fn escape_dismisses_a_prompt_a_press_opened() {
        // The other half of the same reason: `escape` is an *overlay* key, bound
        // to no keymap action. A chord resolved through `Keymap::lookup` could
        // never reach it, and an agent that armed a prompt over MCP would leave
        // the app parked until a human intervened. Through the real ladder it
        // dismisses the prompt, so the loop closes.
        let (mut shell, pty) = busy_shell_with_terminal();
        let _ = press_chord(&mut shell, &mod_spec("w"));
        assert_eq!(
            press_chord(&mut shell, "escape"),
            PressStep::Overlay("tab-close-confirm".to_owned())
        );
        assert_eq!(shell.closing, None, "escape must dismiss the prompt");
        assert_eq!(pty.kill_count(), 0, "dismissing kills nothing");
        // With the prompt gone the app takes chords again.
        assert_eq!(
            press_chord(&mut shell, &mod_spec("d")),
            PressStep::Ran("split-vertical".to_owned())
        );
    }

    #[test]
    fn enter_confirms_a_prompt_a_press_opened() {
        // The affirmative half: an agent can carry the close it asked for
        // through to the kill, without a human answering the prompt.
        let (mut shell, pty) = busy_shell_with_terminal();
        let _ = press_chord(&mut shell, &mod_spec("w"));
        let _ = press_chord(&mut shell, "enter");
        assert_eq!(pty.kill_count(), 1, "enter must confirm the close");
    }

    #[test]
    fn a_named_action_runs_without_going_through_the_keymap() {
        // The behaviour half: `run_action` names the action directly, so it
        // keeps working after a rebind that would break the chord.
        let (mut shell, _pty) = shell_with_terminal();
        let mut keymap = Keymap::defaults();
        keymap.set(Action::SplitVertical, [KeyChord::new("F13", 0)]);
        shell.keymap = keymap;
        let (outcome, _task) = shell.perform_presses(vec![Press::Command(Action::SplitVertical)]);
        assert_eq!(
            outcome.steps,
            vec![PressStep::Ran("split-vertical".to_owned())]
        );
        assert_eq!(
            shell.core.workspace.tabs[0].sessions().len(),
            2,
            "the action runs even though its chord is now unreachable"
        );
    }

    /// The `inert` step for an action name, with its reason — the shape these
    /// tests assert over and over.
    fn inert(action: &str, reason: &'static str) -> PressStep {
        PressStep::Inert {
            action: action.to_owned(),
            reason,
        }
    }

    #[test]
    fn an_action_with_no_surface_yet_reports_inert_not_ran() {
        // `open-new-session` is in the keymap vocabulary and still unwired.
        // Reporting `ran` would have an agent record a gesture that never
        // happened — and, verifying a fix, read a false pass. `inert` also says
        // *don't retry*, which `unbound` would not: no rebinding will help.
        let (mut shell, pty) = shell_with_terminal();
        let (outcome, _task) = shell.perform_presses(vec![Press::Command(Action::OpenNewSession)]);
        assert_eq!(outcome.steps, vec![inert("open-new-session", "no-surface")]);
        assert_eq!(shell.core.workspace.tabs.len(), 1, "nothing was opened");
        assert_eq!(pty.spawn_count(), 1, "and nothing was spawned");
    }

    #[test]
    fn an_action_that_refused_for_want_of_context_says_so() {
        // Found in live testing: on an empty workspace the launch chords do
        // nothing, because `new_claude_here` has no focused session to derive a
        // repo from. It used to report `ran`, so an agent — like a human pressing
        // the chord — would conclude a session had opened.
        //
        // `no-context` is not `no-surface`: the caller can *create* the missing
        // precondition and try again, where an unwired action will never work.
        let (mut shell, pty) = empty_shell();
        let (outcome, _task) =
            shell.perform_presses(vec![Press::Command(Action::NewClaudeSessionHere)]);
        assert_eq!(
            outcome.steps,
            vec![inert("new-claude-session-here", "no-context")]
        );
        assert_eq!(pty.spawn_count(), 0, "nothing was spawned");
    }

    #[test]
    fn every_action_that_can_refuse_reports_which_kind_of_nothing() {
        // The five handlers that bail out before acting, each on its own missing
        // precondition. Pinned together because the *set* is the contract: an
        // action added to `run_action` that can refuse and does not say so
        // reintroduces the false pass this distinction exists to kill.
        let (mut shell, _pty) = empty_shell();
        for (action, name) in [
            (Action::NewClaudeSessionHere, "new-claude-session-here"),
            (Action::ReopenClosedTab, "reopen-closed-tab"),
            (Action::ScrollTop, "scroll-top"),
            (Action::ScrollBottom, "scroll-bottom"),
            (Action::NextTab, "next-tab"),
            (Action::PrevTab, "prev-tab"),
            // There is no tab to close, so `request_close` bails on its range
            // check — and `close-focused` is the action whose prompt-arming the
            // headline overlay test depends on, so a false `ran` here is the
            // worst of the set.
            (Action::CloseFocused, "close-focused"),
            // Nothing is selected, so there is nothing to put on the clipboard.
            // Told `ran`, an agent would follow with `paste` and paste whatever
            // was on the clipboard before.
            (Action::Copy, "copy"),
        ] {
            let (outcome, _task) = shell.perform_presses(vec![Press::Command(action)]);
            assert_eq!(
                outcome.steps,
                vec![inert(name, "no-context")],
                "{name} refused, so it must not report `ran`"
            );
        }
    }

    #[test]
    fn copy_runs_only_with_something_selected() {
        // Three states, because the negative case alone leaves the guard
        // untested: mutation testing survived `copy_selection` always refusing
        // *and* both settings of `!sel.is_empty()` until the positive case and
        // the empty-string case were pinned alongside it.
        //
        // The clipboard write itself is an iced task and not observable here, so
        // the verdict is the assertion — which is exactly what a caller reads.
        let (mut shell, _pty) = shell_with_terminal();

        let (nothing, _task) = shell.perform_presses(vec![Press::Command(Action::Copy)]);
        assert_eq!(
            nothing.steps,
            vec![inert("copy", "no-context")],
            "no selection at all"
        );

        shell.selection = Some(String::new());
        let (empty, _task) = shell.perform_presses(vec![Press::Command(Action::Copy)]);
        assert_eq!(
            empty.steps,
            vec![inert("copy", "no-context")],
            "an empty selection is nothing to copy either"
        );

        shell.selection = Some("cargo test".to_owned());
        let (text, _task) = shell.perform_presses(vec![Press::Command(Action::Copy)]);
        assert_eq!(
            text.steps,
            vec![PressStep::Ran("copy".to_owned())],
            "real text on the clipboard is a copy that ran"
        );
    }

    #[test]
    fn tab_cycling_runs_and_moves_in_opposite_directions() {
        // The refusal tests only exercise cycling on an *empty* workspace, where
        // reporting nothing is correct — so the positive case was uncovered, and
        // mutation testing proved it twice: making `cycle_tab` always refuse went
        // unnoticed, and so did dropping the sign that tells `prev` from `next`.
        //
        // Three tabs, not two: with two, wrapping makes both directions land on
        // the same tab, so the test could not tell them apart.
        let mut shell = shell_with_three_tabs();
        let tabs = shell.core.workspace.tabs.len();
        let start = shell.core.workspace.active;

        let (next, _task) = shell.perform_presses(vec![Press::Command(Action::NextTab)]);
        assert_eq!(next.steps, vec![PressStep::Ran("next-tab".to_owned())]);
        assert_eq!(
            shell.core.workspace.active,
            (start + 1) % tabs,
            "next-tab moves forward"
        );

        let (back, _task) = shell.perform_presses(vec![Press::Command(Action::PrevTab)]);
        assert_eq!(back.steps, vec![PressStep::Ran("prev-tab".to_owned())]);
        assert_eq!(
            shell.core.workspace.active, start,
            "prev-tab undoes it, so the two are not the same action"
        );

        let (again, _task) = shell.perform_presses(vec![Press::Command(Action::PrevTab)]);
        assert_eq!(again.steps, vec![PressStep::Ran("prev-tab".to_owned())]);
        assert_eq!(
            shell.core.workspace.active,
            (start + tabs - 1) % tabs,
            "and it wraps backwards, not forwards"
        );
    }

    #[test]
    fn a_record_toggle_blocked_by_a_draining_screencast_is_inert() {
        // The fifth refusal, which needs a recorder mid-drain rather than an
        // empty workspace: a ⌘⇧R that lands while a finish is pending on
        // in-flight frames is ignored so it can't replace the recorder under the
        // encoder — and that must not read as "recording started".
        let (mut shell, _pty) = shell_with_terminal();
        let (idle, _task) = shell.perform_presses(vec![Press::Command(Action::ToggleRecord)]);
        assert_eq!(
            idle.steps,
            vec![PressStep::Ran("toggle-record".to_owned())],
            "an idle shell accepts the toggle"
        );
        // Force the drain the same way `a_toggle_is_blocked_while_the_previous_
        // recording_drains` does: the real state needs in-flight screenshots.
        shell.record.finish_pending = true;
        shell.record.inflight = 1;
        let (blocked, _task) = shell.perform_presses(vec![Press::Command(Action::ToggleRecord)]);
        assert_eq!(blocked.steps, vec![inert("toggle-record", "no-context")]);
    }

    #[test]
    fn new_shell_here_never_refuses_because_it_falls_back_to_home() {
        // The counterexample that keeps the line honest: this handler *has* a
        // fallback, so it acts from an empty workspace and reports `ran`. Only a
        // handler that genuinely bails out is inert.
        let (mut shell, pty) = empty_shell();
        let (outcome, _task) = shell.perform_presses(vec![Press::Command(Action::NewShellHere)]);
        assert_eq!(
            outcome.steps,
            vec![PressStep::Ran("new-shell-here".to_owned())]
        );
        assert_eq!(pty.spawn_count(), 1, "it launched in the home directory");
    }

    #[test]
    fn a_surfaced_action_whose_effect_is_nothing_still_reports_ran() {
        // The other side of that line: `activate-tab-9` on a one-tab workspace
        // is absorbed by core as a no-op, but the action *is* wired — so it
        // reports `ran`. `inert` is about a missing surface, not a quiet effect,
        // and collapsing the two would make the distinction useless.
        let (mut shell, _pty) = shell_with_terminal();
        let (outcome, _task) = shell.perform_presses(vec![Press::Command(Action::ActivateTab(8))]);
        assert_eq!(
            outcome.steps,
            vec![PressStep::Ran("activate-tab-9".to_owned())]
        );
    }

    #[test]
    fn an_open_prompt_gates_a_named_action_too() {
        // `run_action` skips the keymap but not the ladder: letting it act
        // through a prompt would give the MCP surface a reach the keyboard has
        // not got, which is the one thing this control surface must never do.
        let (mut shell, _pty) = busy_shell_with_terminal();
        let _ = shell.update(Message::RequestCloseTab(0));
        let (outcome, _task) = shell.perform_presses(vec![Press::Command(Action::SplitVertical)]);
        assert_eq!(
            outcome.steps,
            vec![PressStep::Overlay("tab-close-confirm".to_owned())]
        );
        assert_eq!(
            shell.core.workspace.tabs[0].sessions().len(),
            1,
            "the split must not happen behind the prompt"
        );
    }

    #[test]
    fn every_overlay_that_owns_the_keyboard_names_itself() {
        // The ladder has one reader too many to leave unpinned: the key router,
        // the terminal-input guard, and this control surface. Each overlay must
        // both claim the keyboard and report a distinct name, or a caller
        // reading `overlay` cannot tell which prompt is in its way.
        let (mut shell, _pty) = busy_shell_with_terminal();
        assert!(
            shell.keyboard_owner().is_none(),
            "a plain shell owns nothing"
        );
        assert!(shell.accepts_terminal_input(), "and takes terminal input");

        let _ = shell.update(Message::RequestCloseTab(0));
        let owner = shell
            .keyboard_owner()
            .expect("the prompt owns the keyboard");
        assert_eq!(owner.label(), "tab-close-confirm");
        assert!(
            !shell.accepts_terminal_input(),
            "an owned keyboard sends nothing to the terminal — the guard and the \
             ladder must agree"
        );
    }

    #[test]
    fn a_drag_selection_reaches_the_pty_as_grid_anchored_ops() {
        // The canvas turns a press-then-drag into Select ops; the shell must
        // route them through core to the PTY, which owns the grid selection so
        // the highlight follows the text through scroll.
        let (mut shell, pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::Select {
            session,
            op: SelectOp::Start {
                line: 0,
                col: 0,
                side: SelectSide::Left,
            },
        });
        let _ = shell.update(Message::Select {
            session,
            op: SelectOp::Update {
                line: 0,
                col: 5,
                side: SelectSide::Right,
            },
        });
        assert_eq!(
            pty.selects(),
            vec![
                SelectOp::Start {
                    line: 0,
                    col: 0,
                    side: SelectSide::Left
                },
                SelectOp::Update {
                    line: 0,
                    col: 5,
                    side: SelectSide::Right
                },
            ],
            "press and drag reach the PTY as grid-anchored selection ops"
        );
    }

    #[test]
    fn a_double_click_selects_the_word_and_copies_it() {
        // SelectAndCopy sets the native word selection *and* lands the text on
        // the clipboard in one step.
        let (mut shell, pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::SelectAndCopy {
            session,
            op: SelectOp::Range {
                line0: 0,
                col0: 4,
                line1: 0,
                col1: 14,
            },
            text: "src/main.rs".to_string(),
        });
        assert_eq!(
            pty.selects(),
            vec![SelectOp::Range {
                line0: 0,
                col0: 4,
                line1: 0,
                col1: 14
            }],
            "the word range is applied to the terminal selection"
        );
        assert_eq!(
            shell.selection.as_deref(),
            Some("src/main.rs"),
            "the word is remembered as the last copy"
        );
    }

    #[test]
    fn a_drag_release_asks_the_pty_to_copy_its_live_selection() {
        // The copy reads the terminal's own selection (returned out-of-band), so
        // it is exact even right after a fast drag — not the possibly-lagged
        // snapshot the shell last rendered.
        let (mut shell, pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::RequestCopySelection { session });
        assert_eq!(
            pty.copy_count(),
            1,
            "a drag release asks the terminal to copy its selection"
        );
    }

    #[test]
    fn a_right_click_paste_lands_in_the_pane_under_the_pointer() {
        // The right-click carries its own session because the pane you point at
        // need not be the focused one — a paste into the wrong split is worse
        // than no paste at all.
        let (mut shell, pty) = shell_with_terminal();
        let pointed = shell.core.workspace.focused_session().expect("focused");
        let (split, _task) = shell.perform_action(BridgeAction::Split {
            pane: None,
            dir: SplitDir::Vertical,
        });
        assert_eq!(split.error, None);
        let elsewhere = shell.core.workspace.focused_session().expect("focused");
        assert_ne!(elsewhere, pointed, "the split moved focus off the target");

        let _ = shell.update(Message::PasteInto {
            session: pointed,
            content: Some("hello".to_string()),
        });
        assert_eq!(
            pty.writes_seen(),
            vec![(pointed, b"hello".to_vec())],
            "the clipboard reached the pointed pane, and only it"
        );
    }

    /// A shell whose focused terminal shows a highlighted selection — what the
    /// copy chord must read when copy-on-select is off and nothing has filled
    /// the cache.
    fn shell_with_a_visible_selection() -> (Shell, Arc<RecordingPty>, SessionId) {
        let (mut shell, pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let mut screen = screen_of("cargo test");
        screen.selection = vec![(0, 0, 4)];
        shell.screens.insert(session, screen);
        (shell, pty, session)
    }

    #[test]
    fn the_copy_chord_reads_a_mouse_selection_when_copy_on_select_is_off() {
        // With the gesture off nothing fills the selection cache, so a cache-only
        // copy would leave a dragged selection uncopyable by any means — worse
        // than before the setting existed.
        let (mut shell, pty, _session) = shell_with_a_visible_selection();
        let (verdict, _task) = shell.perform_presses(vec![Press::Command(Action::Copy)]);
        assert_eq!(
            verdict.steps,
            vec![PressStep::Ran("copy".to_owned())],
            "a highlighted selection is something to copy"
        );
        assert_eq!(
            pty.copy_count(),
            1,
            "the chord asks the terminal for its live selection"
        );
    }

    #[test]
    fn the_copy_chord_prefers_the_live_selection_to_the_last_copied_text() {
        // The cache holds what was last *copied*, not what is selected now.
        // Reading it first would put stale text on the clipboard while a fresh
        // highlight sits on screen — a silent wrong answer, the worst kind.
        let (mut shell, pty, _session) = shell_with_a_visible_selection();
        shell.selection = Some("an earlier copy".to_owned());
        let _ = shell.perform_presses(vec![Press::Command(Action::Copy)]);
        assert_eq!(
            pty.copy_count(),
            1,
            "the live selection wins over the cache"
        );
    }

    #[test]
    fn a_right_click_paste_focuses_the_pane_it_pastes_into() {
        // Every terminal focuses the pane its paste-click landed on; the left
        // button already does through `mouse_area`.
        let (mut shell, _pty) = shell_with_terminal();
        let pointed = shell.core.workspace.focused_session().expect("focused");
        let (split, _task) = shell.perform_action(BridgeAction::Split {
            pane: None,
            dir: SplitDir::Vertical,
        });
        assert_eq!(split.error, None);
        assert_ne!(
            shell.core.workspace.focused_session(),
            Some(pointed),
            "the split moved focus off the target"
        );

        let _ = shell.update(Message::RequestPaste { session: pointed });
        assert_eq!(
            shell.core.workspace.focused_session(),
            Some(pointed),
            "the pane you paste into is the pane you are working in"
        );
    }

    #[test]
    fn a_right_click_paste_is_swallowed_by_an_open_prompt() {
        // The prompt that owns the keyboard owns the input: the paste chord is
        // swallowed there, and the pointer must not be a way around it.
        let (mut shell, _pty) = busy_shell_with_terminal();
        let pointed = shell.core.workspace.focused_session().expect("focused");
        let (split, _task) = shell.perform_action(BridgeAction::Split {
            pane: None,
            dir: SplitDir::Vertical,
        });
        assert_eq!(split.error, None);
        let elsewhere = shell.core.workspace.focused_session().expect("focused");
        shell.closing = Some(0);

        let _ = shell.update(Message::RequestPaste { session: pointed });
        assert_eq!(
            shell.core.workspace.focused_session(),
            Some(elsewhere),
            "the gesture did not even move focus past the prompt"
        );
    }

    #[test]
    fn a_right_click_paste_abandons_an_open_session_rename() {
        // Pointing at a terminal and pasting into it is a deliberate move away
        // from the sidebar edit — the same reading the scroll and the keyboard
        // paste already get.
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        shell.renaming = Some(("sid".to_string(), "half-typed".to_string()));
        let _ = shell.update(Message::RequestPaste { session });
        assert!(shell.renaming.is_none(), "the gesture blurs the edit");
    }

    #[test]
    fn a_paste_is_bracketed_exactly_when_its_own_terminal_asked_for_it() {
        // Bracketing is per session, so a paste addressed to a pane must read
        // *that* pane's mode — not the focused pane's.
        let (mut shell, pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let mut screen = screen_of("$ ");
        screen.bracketed_paste = true;
        shell.screens.insert(session, screen);
        let _ = shell.update(Message::PasteInto {
            session,
            content: Some("hi".to_string()),
        });
        assert_eq!(
            pty.writes(),
            vec![termherd_pty::paste_bytes("hi", true)],
            "the terminal asked for bracketed paste and got it"
        );
    }

    #[test]
    fn a_paste_of_nothing_writes_nothing() {
        // An empty clipboard must not send the bracket markers on their own.
        let (mut shell, pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PasteInto {
            session,
            content: None,
        });
        let _ = shell.update(Message::PasteInto {
            session,
            content: Some(String::new()),
        });
        assert!(pty.writes().is_empty());
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

    /// With no browsable projects the sidebar shows a status line, then the
    /// Plans & mémoire section — the branch where the first section's leading
    /// divider is suppressed so it does not underline the status. Render it to
    /// prove that path assembles without panicking.
    #[test]
    fn the_sidebar_renders_a_status_line_above_a_lone_section() {
        let (mut shell, _pty) = shell_with_terminal();
        let _ = shell
            .core
            .apply(termherd_core::Event::ScanCompleted(Vec::new()));
        shell.docs = vec![DocEntry {
            kind: crate::docs::DocKind::Plan,
            label: "PRD.md".to_string(),
            path: std::path::PathBuf::from("/tmp/PRD.md"),
        }];
        assert!(
            shell.core.visible_projects().is_empty(),
            "no scanned sessions should leave the project list empty",
        );
        let _ = shell.view();
    }

    #[test]
    fn resuming_a_known_session_titles_the_tab_with_its_session_name() {
        // Claude reports only its own product name as an OSC title until it
        // has something session-specific to say, and the decoder discards that
        // as naming the program rather than the session — so the live-title
        // override never fires here, and the tab must take the session's name
        // from the scanned digest instead of the generic `project 🤖` label.
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
            selection: Vec::new(),
            default_bg: [0x11, 0x13, 0x18],
            cursor_color: [0xd0, 0xd0, 0xd0],
        }
    }

    #[test]
    fn capture_writes_a_json_dump_of_the_whole_workspace() {
        // a capture writes capture-<ts>.json holding the whole workspace —
        // focus, config, sidebar, tabs — plus the focused terminal's visible
        // text. Driven through the `perform_capture` dir seam so it lands in a
        // tempdir, not the real home; the PNG is an async iced screenshot and is
        // not exercised here.
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        shell.screens.insert(session, screen_of("$ cargo test"));

        // Build the dump through the same seams `capture()` uses.
        let inputs = shell.snapshot_inputs_for(&SnapshotFilter::capture());
        let dump = shell.core.build_capture(&inputs);

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
        assert_eq!(json["focus"]["tab"], 0);
        assert_eq!(json["focus"]["session"], session.0.get().to_string());
        assert_eq!(json["tabs"][0]["title"], "project $");
        assert_eq!(
            json["tabs"][0]["panes"][0]["handle"],
            session.0.get().to_string()
        );
        assert_eq!(
            json["terminals"][session.0.get().to_string()],
            "$ cargo test",
            "the focused pane's screen rides in the dump"
        );
        assert!(
            json["config"].is_object(),
            "a capture carries the resolved config, not just the terminal"
        );
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
        let _ = shell.update(Message::PtyExited {
            session,
            clean: false,
        });
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
        assert_eq!(shell.core.workspace.tab_of(anchor), Some(1));

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
    fn escape_abandons_a_session_rename_without_touching_the_title() {
        // The sidebar's rename cancels on blur, where a tab's commits — so
        // escape must discard the edit, matching the gesture it stands in for.
        let (mut shell, _pty) = shell_with_terminal();
        browse_named(&mut shell, "sid", "/p", "derived summary", None);
        let record = shell.core.sidebar.projects[0].sessions[0].clone();
        let derived = shell.core.session_title(&record);

        shell.renaming = Some(("sid".to_string(), "half-typed".to_string()));
        let _ = shell.on_key(press(Key::Named(Named::Escape), Modifiers::default(), None));

        assert!(shell.renaming.is_none(), "Escape abandons the edit");
        assert_eq!(
            shell.core.session_title(&record),
            derived,
            "an abandoned rename leaves the derived title intact"
        );
    }

    #[test]
    fn escape_frees_the_control_surface_from_a_session_rename() {
        // The reason this bug matters beyond the sidebar: every press answers
        // `overlay` while the field is open, and no press could clear it. The
        // verdict stays `overlay` — the rename did consume the key — but the
        // press after it must find the ladder empty.
        let (mut shell, _pty) = shell_with_terminal();
        shell.renaming = Some(("sid".to_string(), "half-typed".to_string()));

        assert_eq!(
            press_chord(&mut shell, "escape"),
            PressStep::Overlay("session-rename".to_owned()),
            "the rename consumes the key it acts on, like every other overlay"
        );
        assert!(shell.keyboard_owner().is_none(), "and then lets go");
        assert_eq!(
            press_chord(&mut shell, &mod_spec("d")),
            PressStep::Ran("split-vertical".to_owned()),
            "the surface takes chords again"
        );
    }

    #[test]
    fn every_overlay_that_owns_the_keyboard_can_be_left_from_the_keyboard() {
        // The invariant this bug broke, stated once for the whole ladder rather
        // than per prompt: an overlay that takes the keyboard and answers no key
        // parks the app for anyone without a mouse — which is every MCP caller.
        // Driven off `KeyboardOwner::ALL`, so a rung added without an exit fails
        // here instead of shipping. Failures accumulate rather than stopping at
        // the first: a sweep that names one offender reads as "and the rest are
        // fine", which is the assumption that let this one through.
        //
        // What it does *not* claim: that leaving is harmless. It asserts the
        // owner let go, not what letting go cost — the doc editor satisfies it
        // while discarding unsaved edits. A rung whose exit destroys state
        // needs its own test saying so; this one only rules out the parking.
        let mut swallowed = Vec::new();
        for owner in KeyboardOwner::ALL {
            let (mut shell, _pty) = shell_with_terminal();
            arm_overlay(&mut shell, owner);
            assert_eq!(
                shell.keyboard_owner().map(KeyboardOwner::label),
                Some(owner.label()),
                "{} must be armed before the sweep means anything",
                owner.label()
            );

            let _ = shell.on_key(press(Key::Named(Named::Escape), Modifiers::default(), None));

            if shell.keyboard_owner().is_some() {
                swallowed.push(owner.label());
            }
        }
        assert!(
            swallowed.is_empty(),
            "these overlays swallow escape and cannot be left from the keyboard: {swallowed:?}"
        );
    }

    #[test]
    fn only_escape_leaves_an_overlay_and_an_ordinary_key_does_not() {
        // The other half of the sweep above, and not a formality: an exit that
        // fires on *every* key is worse than none, since typing into a rename
        // field would discard the edit at the first letter. Mutation testing
        // named this gap — the escape test alone cannot tell the two apart.
        let mut dismissed = Vec::new();
        for owner in KeyboardOwner::ALL {
            let (mut shell, _pty) = shell_with_terminal();
            arm_overlay(&mut shell, owner);

            let _ = shell.on_key(press(
                Key::Character("x".into()),
                Modifiers::default(),
                Some("x"),
            ));

            if shell.keyboard_owner().is_none() {
                dismissed.push(owner.label());
            }
        }
        assert!(
            dismissed.is_empty(),
            "these overlays let an ordinary key dismiss them: {dismissed:?}"
        );
    }

    /// Put `shell` into the state `owner` names.
    ///
    /// The `match` is what makes the sweep above honest: a new `KeyboardOwner`
    /// variant fails to compile here rather than quietly escaping it.
    fn arm_overlay(shell: &mut Shell, owner: KeyboardOwner) {
        match owner {
            KeyboardOwner::TabRename => {
                let current = shell.core.workspace.tabs[0].display_title().to_owned();
                let _ = shell.update(Message::StartTabRename { index: 0, current });
            }
            KeyboardOwner::SessionRename => {
                let _ = shell.update(Message::StartRename {
                    session: "sid".to_string(),
                    current: "half-typed".to_string(),
                });
            }
            KeyboardOwner::Quit => shell.closing_window = Some(window::Id::unique()),
            KeyboardOwner::TabClose(index) => shell.closing = Some(index),
            KeyboardOwner::Archive => shell.archiving = Some("sess".to_string()),
            KeyboardOwner::Doc => {
                let _ = shell.update(Message::DocLoaded {
                    label: "CLAUDE.md".to_string(),
                    path: PathBuf::from("/nowhere/CLAUDE.md"),
                    content: "notes".to_string(),
                    mtime: None,
                });
            }
        }
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
            .tab_of(anchor)
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
        // The launch resizes the lone pane once; read the visible-sidebar cols
        // straight off that recorded resize rather than an internal helper.
        let cols_visible = pty
            .resizes()
            .last()
            .map(|(cols, _)| *cols)
            .expect("the launch resizes the pane once");
        let resizes_before = pty.resizes().len();

        let _ = shell.toggle_sidebar();
        assert!(shell.core.sidebar.hidden, "toggle should hide the sidebar");

        let resizes = pty.resizes();
        assert!(
            resizes.len() > resizes_before,
            "toggling the sidebar must resize the PTY"
        );
        let cols_hidden = resizes
            .last()
            .map(|(cols, _)| *cols)
            .expect("the toggle resizes the pane");
        assert!(
            cols_hidden > cols_visible,
            "hiding the sidebar must add columns (was {cols_visible}, now {cols_hidden})"
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

    #[test]
    fn split_action_opens_a_new_pane_beside_the_focused_one() {
        // The `split-*` keymap action must reach `core` (it is dropped on the
        // floor today), minting a second session in the active tab and spawning
        // its PTY while the original pane stays open.
        let (mut shell, pty) = shell_with_terminal();
        let spawns_before = pty.spawn_count();
        let original = shell
            .core
            .workspace
            .focused_session()
            .expect("a launched terminal is focused");

        let _ = shell.run_action(Action::SplitVertical);

        let active = shell.core.workspace.active;
        let sessions = shell.core.workspace.tabs[active].sessions();
        assert_eq!(
            sessions.len(),
            2,
            "splitting must add a second pane to the active tab"
        );
        assert!(
            sessions.contains(&original),
            "the original pane stays open beside the new one"
        );
        assert_eq!(
            pty.spawn_count(),
            spawns_before + 1,
            "the new pane spawns its own PTY"
        );
    }

    #[test]
    fn close_focused_closes_only_the_focused_pane_in_a_split() {
        // `mod+w` in a split must collapse just the focused pane, not the whole
        // tab: the sibling survives, the tab stays open, one PTY is killed.
        let (mut shell, pty) = shell_with_terminal();
        let original = shell
            .core
            .workspace
            .focused_session()
            .expect("a launched terminal is focused");
        let _ = shell.run_action(Action::SplitVertical);
        let new_pane = shell
            .core
            .workspace
            .focused_session()
            .expect("the new split pane is focused");
        let active = shell.core.workspace.active;
        assert_eq!(shell.core.workspace.tabs[active].sessions().len(), 2);

        let kills_before = pty.kill_count();
        let _ = shell.run_action(Action::CloseFocused);

        assert_eq!(shell.core.workspace.tabs.len(), 1, "the tab stays open");
        let active = shell.core.workspace.active;
        let sessions = shell.core.workspace.tabs[active].sessions();
        assert_eq!(sessions, vec![original], "only the original pane remains");
        assert!(
            !sessions.contains(&new_pane),
            "the focused pane was the one closed"
        );
        assert_eq!(
            pty.kill_count(),
            kills_before + 1,
            "exactly the closed pane's PTY is killed"
        );
    }

    #[test]
    fn focus_prev_returns_to_the_original_pane_after_a_split() {
        // Splitting focuses the new pane; `focus-prev` must cycle back — the
        // `focus-*` actions are dropped on the floor today.
        let (mut shell, _pty) = shell_with_terminal();
        let original = shell
            .core
            .workspace
            .focused_session()
            .expect("a launched terminal is focused");

        let _ = shell.run_action(Action::SplitVertical);
        let new_pane = shell
            .core
            .workspace
            .focused_session()
            .expect("the new split pane is focused");
        assert_ne!(
            new_pane, original,
            "a split focuses the freshly minted pane"
        );

        let _ = shell.run_action(Action::FocusPrev);
        assert_eq!(
            shell.core.workspace.focused_session(),
            Some(original),
            "focus-prev walks back to the original pane"
        );
    }

    #[test]
    fn a_window_resize_resizes_every_split_pane() {
        // Per-leaf PTY geometry: a resize must size *each* pane to its own
        // sub-rect, not just the focused one — two panes, two resizes.
        let (mut shell, pty) = shell_with_terminal();
        let _ = shell.run_action(Action::SplitVertical);
        let resizes_before = pty.resizes().len();

        let _ = shell.update(Message::Window(
            window::Id::unique(),
            window::Event::Resized(Size::new(1600.0, 900.0)),
        ));

        assert!(
            pty.resizes().len() >= resizes_before + 2,
            "each of the two split panes must be resized to its own geometry \
             (was {resizes_before}, now {})",
            pty.resizes().len()
        );
    }

    /// A `Shell` with no terminal open (empty workspace), plus its recording
    /// PTY — for the "new shell here" empty-workspace path.
    fn empty_shell() -> (Shell, Arc<RecordingPty>) {
        let pty = Arc::new(RecordingPty::default());
        let (_tx, rx) = iced::futures::channel::mpsc::unbounded::<PtyEvent>();
        let shell = Shell::new(
            WindowConfig::default(),
            test_ports(pty.clone(), rx),
            test_live_bridge(),
            test_startup(),
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

    /// A `SpawnSpec` for `session` running `launch`, with no mcp config yet —
    /// the shape `attach_mcp` receives from `core`.
    fn bare_spawn(session: SessionId, launch: Launch) -> SpawnSpec {
        SpawnSpec {
            session,
            cwd: None,
            launch,
            cols: 80,
            rows: 24,
            mcp: None,
        }
    }

    fn session_id(n: u64) -> SessionId {
        SessionId(std::num::NonZeroU64::new(n).expect("nonzero session id"))
    }

    #[test]
    fn a_claude_spawn_gets_an_mcp_config_and_a_revocable_token() {
        let (mut shell, _pty) = empty_shell();
        shell.mcp_endpoint = Some(crate::mcp::Endpoint {
            url: "http://127.0.0.1:9/mcp".into(),
        });
        let session = session_id(1);
        let mut spec = bare_spawn(session, Launch::Claude { resume: None });

        shell.attach_mcp(&mut spec);

        let config = spec.mcp.expect("a Claude spawn carries an mcp config");
        assert_eq!(
            config.url, "http://127.0.0.1:9/mcp",
            "the endpoint url is injected"
        );
        assert!(!config.token.is_empty(), "a per-session token is minted");
        assert!(
            shell.mcp_session_tokens.contains_key(&session),
            "the token is remembered for later revocation"
        );

        shell.revoke_mcp(session);
        assert!(
            !shell.mcp_session_tokens.contains_key(&session),
            "killing the session forgets its token"
        );
    }

    #[test]
    fn each_claude_spawn_gets_a_distinct_token() {
        let (mut shell, _pty) = empty_shell();
        shell.mcp_endpoint = Some(crate::mcp::Endpoint {
            url: "http://127.0.0.1:9/mcp".into(),
        });
        let mut first = bare_spawn(session_id(1), Launch::Claude { resume: None });
        let mut second = bare_spawn(session_id(2), Launch::Claude { resume: None });
        shell.attach_mcp(&mut first);
        shell.attach_mcp(&mut second);
        assert_ne!(
            first.mcp.expect("first token").token,
            second.mcp.expect("second token").token,
            "two sessions never share a token"
        );
    }

    #[test]
    fn a_shell_spawn_never_gets_an_mcp_config() {
        let (mut shell, _pty) = empty_shell();
        shell.mcp_endpoint = Some(crate::mcp::Endpoint {
            url: "http://127.0.0.1:9/mcp".into(),
        });
        let mut spec = bare_spawn(session_id(1), Launch::Shell);
        shell.attach_mcp(&mut spec);
        assert!(spec.mcp.is_none(), "a plain shell never reaches the bridge");
    }

    #[test]
    fn no_bound_server_means_no_mcp_config() {
        // The server failed to bind (or no runtime): `mcp_endpoint` is `None`, so
        // even a Claude launch goes out without the bridge rather than panicking.
        let (mut shell, _pty) = empty_shell();
        let mut spec = bare_spawn(session_id(1), Launch::Claude { resume: None });
        shell.attach_mcp(&mut spec);
        assert!(spec.mcp.is_none(), "no endpoint → no injection");
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
        assert_eq!(
            shell.core.live_session_count(),
            1,
            "a launched session is live"
        );
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyExited {
            session,
            clean: false,
        });
        assert_eq!(
            shell.core.live_session_count(),
            0,
            "an exited session no longer counts as live to kill"
        );
    }

    /// Serialises the tests that spawn a **real** PTY. Session ids restart at 1
    /// with each `Shell`, and the shell-integration files live in
    /// `$TMPDIR/termherd-shell-<id>` — so two of these running at once both
    /// claim session 1's directory, and whichever writes second deletes the
    /// other's startup file out from under a shell that has not read it yet.
    /// The victim then runs unintegrated, announces nothing, and fails on its
    /// deadline looking like a code regression. Production is spared by the
    /// single-instance lock, which is why this guard belongs here and not in
    /// the adapter.
    static REAL_PTY: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Take [`REAL_PTY`], surviving a sibling that panicked while holding it —
    /// the lock orders these tests, it does not protect data.
    fn one_real_pty_at_a_time() -> std::sync::MutexGuard<'static, ()> {
        REAL_PTY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// End-to-end auto-close: a **real PTY** running the platform default
    /// shell, a typed `exit`, and the real `update` loop — everything but the
    /// iced runtime (whose event glue, [`streams::pty_message`], this test
    /// shares). Regression guard for the ConPTY gap where a child's natural
    /// exit never surfaced as reader EOF, so the tab silently stayed open.
    #[test]
    fn typing_exit_into_a_real_shell_closes_the_tab_end_to_end() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let _serialised = one_real_pty_at_a_time();

        let (tx, rx) = mpsc::channel::<PtyEvent>();
        let sink: termherd_pty::EventSink = Arc::new(move |ev| {
            let _ = tx.send(ev);
        });
        let pty = Arc::new(termherd_pty::PtyManager::new(
            sink,
            None,
            termherd_pty::Palette::default(),
        ));
        let mut shell = shell_over(pty.clone());
        let cwd = std::env::temp_dir().to_string_lossy().into_owned();
        let _ = shell.launch(cwd, Launch::Shell);
        let session = shell.core.workspace.focused_session().expect("focused");

        pty.write(session, b"exit\r\n").expect("type exit");

        // Pump the adapter's events through the real update loop until the
        // auto-close lands (or the deadline proves it never does).
        let deadline = Instant::now() + Duration::from_secs(20);
        while !shell.core.workspace.tabs.is_empty() && Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(event) => {
                    let _ = shell.update(streams::pty_message(event));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        assert!(
            shell.core.workspace.tabs.is_empty(),
            "typing `exit` into a real shell must auto-close its tab"
        );
        assert!(pty.is_empty(), "the dead session's PTY entry is released");
    }

    /// Whether `program` is on the `PATH` as an executable file — what decides
    /// whether the end-to-end test below has a shell to measure. Extensionless
    /// on purpose: the shells with a recipe are all Unix ones, and a host
    /// without them is exactly the host that must skip.
    fn on_path(program: &str) -> bool {
        std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
    }

    /// End-to-end directory tracking: a **real PTY** running a real shell, a
    /// typed `cd`, and the real `update` loop. The only test that proves the
    /// integration recipe reaches a shell at all — everything below it asserts
    /// a snippet or a decoded string, neither of which can tell whether the
    /// shell ran the hook.
    ///
    /// The shell is **pinned to `bash`** rather than left to the host's login
    /// shell: the recipe covers zsh, bash and fish, so a host running `dash`,
    /// `nu` or bare `/bin/sh` would announce nothing by design and the loop
    /// below could only burn its deadline — failing as if the code had broken.
    /// The precondition is therefore "a shell with a recipe is installed", not
    /// "this is Unix", and the skip tests exactly that. It is a runtime check,
    /// never a `#[cfg]`: the body must compile on every platform, which is the
    /// whole point of the OS-cfg quarantine.
    #[test]
    fn a_cd_in_a_real_shell_moves_the_session_end_to_end() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        if !on_path("bash") {
            return;
        }
        let _serialised = one_real_pty_at_a_time();

        let (tx, rx) = mpsc::channel::<PtyEvent>();
        let sink: termherd_pty::EventSink = Arc::new(move |ev| {
            let _ = tx.send(ev);
        });
        let pty = Arc::new(termherd_pty::PtyManager::new(
            sink,
            Some(termherd_pty::Shell {
                program: "bash".to_owned(),
                args: Vec::new(),
            }),
            termherd_pty::Palette::default(),
        ));
        let mut shell = shell_over(pty.clone());
        // Named for this process: two concurrent runs must not race over one
        // directory, and the run that made it is the run that removes it. The
        // `%` is deliberate — it is the one character the shell escapes on the
        // way out and the decoder resolves on the way in, so a real directory
        // called `…100%20…` must not come back with a space in it.
        let leaf = format!("termherd-cwd-e2e-{}-100%20", std::process::id());
        let root = std::env::temp_dir();
        let elsewhere = root.join(&leaf);
        std::fs::create_dir_all(&elsewhere).expect("a directory to cd into");
        let _ = shell.launch(root.to_string_lossy().into_owned(), Launch::Shell);
        let session = shell.core.workspace.focused_session().expect("focused");

        pty.write(session, format!("cd '{leaf}'\r\n").as_bytes())
            .expect("type the cd");

        let landed = |shell: &Shell| focused_cwd(shell).is_some_and(|cwd| cwd.ends_with(&leaf));
        let deadline = Instant::now() + Duration::from_secs(20);
        while !landed(&shell) && Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(event) => {
                    let _ = shell.update(streams::pty_message(event));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let _ = pty.kill(session);
        let outcome = focused_cwd(&shell);
        std::fs::remove_dir_all(&elsewhere).ok();
        assert!(
            outcome.as_deref().is_some_and(|cwd| cwd.ends_with(&leaf)),
            "a `cd` in a real shell must move the session, got {outcome:?}"
        );
    }

    #[test]
    fn a_clean_shell_exit_auto_closes_its_tab() {
        let (mut shell, pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        shell.screens.insert(session, screen_of("$ exit"));
        let _ = shell.update(Message::PtyExited {
            session,
            clean: true,
        });
        assert!(
            shell.core.workspace.tabs.is_empty(),
            "the tab closes by itself on a clean shell exit"
        );
        assert!(
            !shell.screens.contains_key(&session),
            "the cached screen is dropped with its session"
        );
        assert_eq!(
            pty.kill_count(),
            1,
            "the dead session's PTY handles are released"
        );
    }

    #[test]
    fn a_dirty_exit_keeps_the_dead_tab_on_screen() {
        let (mut shell, pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyExited {
            session,
            clean: false,
        });
        assert_eq!(
            shell.core.workspace.tabs.len(),
            1,
            "a failed exit's last screen stays readable"
        );
        assert_eq!(pty.kill_count(), 0);
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
        let _ = shell.update(Message::PtyExited {
            session,
            clean: false,
        });
        assert_eq!(
            shell.core.live_session_count(),
            0,
            "precondition: nothing live"
        );

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
            shell.core.live_session_count(),
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
        let _ = shell.update(Message::PtyExited {
            session,
            clean: false,
        });
        assert_eq!(
            shell.core.live_session_count(),
            0,
            "precondition: nothing live"
        );
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
            !shell.record.toggle_blocked(),
            "an idle shell accepts a record toggle"
        );
        // Mid-drain: a finish is pending on in-flight frame screenshots. A new
        // ⌘⇧R must be ignored, not replace the recorder under the encoder.
        shell.record.finish_pending = true;
        shell.record.inflight = 1;
        assert!(
            shell.record.toggle_blocked(),
            "a draining recorder blocks a new toggle"
        );
    }

    // --- The terminal-sync rung: wait_for_status + read_terminal ---

    use super::bridge::{
        Reply as BridgeReply, ReplyPort, Request as BridgeRequest, TerminalRead, WaitOutcome,
    };
    use tokio::sync::oneshot::error::TryRecvError;

    /// Serve `request` through the bridge seam and hand back the caller's reply
    /// receiver. A parked request leaves it empty — that emptiness is the
    /// assertion a wait test needs.
    fn serve(
        shell: &mut Shell,
        request: BridgeRequest,
    ) -> tokio::sync::oneshot::Receiver<BridgeReply> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = shell.serve(request, ReplyPort::new(tx));
        rx
    }

    /// The `WaitOutcome` a served wait answered with, or `None` while it parks.
    fn waited(rx: &mut tokio::sync::oneshot::Receiver<BridgeReply>) -> Option<WaitOutcome> {
        match rx.try_recv() {
            Ok(BridgeReply::Waited(outcome)) => Some(outcome),
            Ok(other) => panic!("expected a Waited reply, got {other:?}"),
            Err(TryRecvError::Empty) => None,
            Err(error) => panic!("the reply channel closed: {error}"),
        }
    }

    /// The `TerminalRead` a served read answered with. A read never parks, so an
    /// empty channel is a failure, not a state.
    fn read_back(rx: &mut tokio::sync::oneshot::Receiver<BridgeReply>) -> TerminalRead {
        match rx.try_recv() {
            Ok(BridgeReply::Terminal(read)) => read,
            Ok(other) => panic!("expected a Terminal reply, got {other:?}"),
            Err(error) => panic!("a read must answer in the same update: {error}"),
        }
    }

    /// Wait for the two statuses `wait_for_status` exists for.
    fn wait_for(session: u64) -> BridgeRequest {
        BridgeRequest::WaitForStatus {
            session,
            targets: vec![SessionStatus::Idle, SessionStatus::Attention],
        }
    }

    #[test]
    fn wait_answers_at_once_when_the_session_already_holds_a_target_status() {
        // A session sitting on the target must not park: an agent that asks
        // "tell me when it's idle" about an idle session would otherwise hang
        // until its own timeout, having already missed the transition.
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyStatus {
            session,
            status: SessionStatus::Idle,
        });

        let mut rx = serve(&mut shell, wait_for(session.0.get()));
        assert_eq!(
            waited(&mut rx),
            Some(WaitOutcome {
                status: Some(SessionStatus::Idle),
                error: None,
            })
        );
    }

    #[test]
    fn wait_parks_until_the_status_change_arrives() {
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyStatus {
            session,
            status: SessionStatus::Busy,
        });

        let mut rx = serve(&mut shell, wait_for(session.0.get()));
        assert_eq!(
            waited(&mut rx),
            None,
            "a busy session leaves the wait parked"
        );

        let _ = shell.update(Message::PtyStatus {
            session,
            status: SessionStatus::Idle,
        });
        assert_eq!(
            waited(&mut rx),
            Some(WaitOutcome {
                status: Some(SessionStatus::Idle),
                error: None,
            }),
            "the status change settles the parked wait"
        );
    }

    #[test]
    fn wait_stays_parked_through_a_non_target_status() {
        // Busy -> Starting is a change, but not the one asked for; waking on it
        // would report "done" while the command is still running.
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyStatus {
            session,
            status: SessionStatus::Busy,
        });
        let mut rx = serve(&mut shell, wait_for(session.0.get()));

        let _ = shell.update(Message::PtyStatus {
            session,
            status: SessionStatus::Starting,
        });
        assert_eq!(waited(&mut rx), None);
    }

    #[test]
    fn wait_ignores_a_status_change_on_another_session() {
        let (mut shell, _pty) = shell_with_terminal();
        let watched = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyStatus {
            session: watched,
            status: SessionStatus::Busy,
        });
        let _ = shell.launch("/tmp/other".to_string(), Launch::Shell);
        let other = shell.core.workspace.focused_session().expect("focused");
        assert_ne!(other, watched);

        let mut rx = serve(&mut shell, wait_for(watched.0.get()));
        let _ = shell.update(Message::PtyStatus {
            session: other,
            status: SessionStatus::Idle,
        });
        assert_eq!(
            waited(&mut rx),
            None,
            "another pane going idle must not settle this wait"
        );
    }

    #[test]
    fn wait_rejects_an_unknown_handle_without_parking() {
        let (mut shell, _pty) = shell_with_terminal();
        let mut rx = serve(&mut shell, wait_for(9_999));
        let outcome = waited(&mut rx).expect("an unknown handle answers immediately");
        assert_eq!(outcome.status, None);
        assert!(
            outcome.error.is_some(),
            "an unknown handle is rejected, not awaited forever"
        );
    }

    #[test]
    fn wait_is_settled_by_an_unclean_pty_exit() {
        // A crashed session emits no status change — only an exit. Without this
        // the caller would wait out its whole bound on a dead terminal.
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyStatus {
            session,
            status: SessionStatus::Busy,
        });
        let mut rx = serve(&mut shell, wait_for(session.0.get()));

        let _ = shell.update(Message::PtyExited {
            session,
            clean: false,
        });
        assert_eq!(
            waited(&mut rx),
            Some(WaitOutcome {
                status: Some(SessionStatus::Exited),
                error: None,
            }),
            "an exit settles the wait with the status it ended on"
        );
    }

    #[test]
    fn a_waiter_whose_caller_gave_up_is_dropped() {
        // The caller timed out and dropped its end; the shell must not keep the
        // entry alive for a session that may never reach the target.
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyStatus {
            session,
            status: SessionStatus::Busy,
        });
        drop(serve(&mut shell, wait_for(session.0.get())));
        assert_eq!(shell.waiters.len(), 1, "the wait parked");

        let _ = shell.update(Message::PtyStatus {
            session,
            status: SessionStatus::Starting,
        });
        assert!(
            shell.waiters.is_empty(),
            "an abandoned waiter is swept on the next status event"
        );
    }

    #[test]
    fn wait_answers_at_once_for_a_session_that_already_exited() {
        // `Exited` is terminal in core — it refuses to overwrite it — so a wait
        // parked on a dead session could never be woken by a status event. It
        // has to be answered on the spot instead.
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyExited {
            session,
            clean: false,
        });

        let mut rx = serve(&mut shell, wait_for(session.0.get()));
        assert_eq!(
            waited(&mut rx),
            Some(WaitOutcome {
                status: Some(SessionStatus::Exited),
                error: None,
            })
        );
        assert!(shell.waiters.is_empty(), "nothing parked");
    }

    #[test]
    fn a_status_still_in_flight_when_the_exit_lands_settles_as_exited() {
        // The PTY task can emit a status the exit overtakes. Core drops it (a
        // dead session stays dead); the wait must report what core recorded,
        // not the message, or the client is told a crashed session went idle.
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyStatus {
            session,
            status: SessionStatus::Busy,
        });
        let mut rx = serve(&mut shell, wait_for(session.0.get()));

        // The exit lands first and settles the waiter...
        let _ = shell.update(Message::PtyExited {
            session,
            clean: false,
        });
        assert_eq!(
            waited(&mut rx).and_then(|outcome| outcome.status),
            Some(SessionStatus::Exited)
        );

        // ...and a late Idle must not resurrect it for a second waiter either.
        let mut late = serve(&mut shell, wait_for(session.0.get()));
        let _ = shell.update(Message::PtyStatus {
            session,
            status: SessionStatus::Idle,
        });
        assert_eq!(
            waited(&mut late).and_then(|outcome| outcome.status),
            Some(SessionStatus::Exited),
            "core kept the session dead, so the wait reports dead"
        );
    }

    #[test]
    fn a_wait_on_a_session_closed_from_the_ui_is_settled_on_the_next_request() {
        // Closing a tab drops its sessions without any PTY exit reaching
        // `update`, so nothing would settle the waiter. The next served request
        // sweeps it rather than leaving the caller parked on a ghost.
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyStatus {
            session,
            status: SessionStatus::Busy,
        });
        let mut rx = serve(&mut shell, wait_for(session.0.get()));
        assert_eq!(waited(&mut rx), None, "parked while busy");

        let effects = shell.core.apply(termherd_core::Event::CloseTab(0));
        let _ = shell.perform(effects);
        assert!(
            !shell.core.sessions.contains_key(&session),
            "the close took the session out of the registry"
        );

        // Any request re-enters `serve`, which sweeps first.
        drop(serve(&mut shell, BridgeRequest::ListSessions));
        assert_eq!(
            waited(&mut rx).and_then(|outcome| outcome.status),
            Some(SessionStatus::Exited)
        );
        assert!(shell.waiters.is_empty());
    }

    #[test]
    fn abandoned_waiters_are_swept_on_a_quiet_workspace() {
        // Without a sweep on the request path, a quiet workspace never drops
        // the waiters of callers that timed out, and the list grows unbounded.
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        let _ = shell.update(Message::PtyStatus {
            session,
            status: SessionStatus::Busy,
        });
        // Ten timed-out waits in a row on a session that never moves: the list
        // must not accumulate them, since each `serve` sweeps before parking.
        for _ in 0..10 {
            drop(serve(&mut shell, wait_for(session.0.get())));
            assert!(
                shell.waiters.len() <= 1,
                "at most the wait just parked, never a backlog"
            );
        }

        drop(serve(&mut shell, BridgeRequest::ListSessions));
        assert!(
            shell.waiters.is_empty(),
            "no status event fired, yet the dead waiters are gone"
        );
    }

    #[test]
    fn read_terminal_returns_the_panes_visible_text() {
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        shell.screens.insert(session, screen_of("$ cargo test"));

        let mut rx = serve(
            &mut shell,
            BridgeRequest::ReadTerminal {
                session: session.0.get(),
                lines: 40,
            },
        );
        assert_eq!(
            read_back(&mut rx),
            TerminalRead {
                text: Some("$ cargo test".to_owned()),
                error: None,
            }
        );
    }

    #[test]
    fn read_terminal_keeps_only_the_requested_trailing_lines() {
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");
        shell
            .screens
            .insert(session, screen_of("one\ntwo\nthree\nfour"));

        let mut rx = serve(
            &mut shell,
            BridgeRequest::ReadTerminal {
                session: session.0.get(),
                lines: 2,
            },
        );
        assert_eq!(
            read_back(&mut rx).text.as_deref(),
            Some("three\nfour"),
            "the read is bounded like a snapshot's text_lines"
        );
    }

    #[test]
    fn read_terminal_reports_no_text_for_a_pane_that_has_not_rendered() {
        // Distinct from an unknown handle: the session is live, its screen just
        // hasn't arrived. An agent should retry, not give up on the handle.
        let (mut shell, _pty) = shell_with_terminal();
        let session = shell.core.workspace.focused_session().expect("focused");

        let mut rx = serve(
            &mut shell,
            BridgeRequest::ReadTerminal {
                session: session.0.get(),
                lines: 40,
            },
        );
        assert_eq!(
            read_back(&mut rx),
            TerminalRead {
                text: None,
                error: None,
            }
        );
    }

    #[test]
    fn read_terminal_rejects_an_unknown_handle() {
        let (mut shell, _pty) = shell_with_terminal();
        let mut rx = serve(
            &mut shell,
            BridgeRequest::ReadTerminal {
                session: 9_999,
                lines: 40,
            },
        );
        let read = read_back(&mut rx);
        assert_eq!(read.text, None);
        assert!(
            read.error.is_some(),
            "an unknown handle is an error, not empty text"
        );
    }
}
