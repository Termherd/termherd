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
//! - [`ime`] — the input-method wrapper that composes dead/accent keys (#34).
//! - [`input`] — keyboard translation (chords / `TermKey` / modifiers).
//! - [`streams`] — the PTY-output and fs-watch subscription sources.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use iced::advanced::widget::{self, operate, operation::focusable};
use iced::futures::channel::mpsc::UnboundedReceiver;
use iced::widget::text_editor;
use iced::{Point, Size, Subscription, Task, Theme, keyboard, window};
use termherd_core::ports::{ProjectScanner, PtyHost};
use termherd_core::workspace::SessionId;
use termherd_core::{
    Action, Effect, Keymap, Launch, LaunchSpec, ScrollTarget, SessionMeta, SessionRecord,
    SessionStatus,
};
use termherd_pty::{PtyEvent, Screen, TermKey};

use crate::docs::DocEntry;
use crate::settings::ThemeChoice;
use crate::window_config::WindowConfig;

mod ime;
mod input;
mod streams;
mod terminal;
mod view;

use input::{chord_of, event_modifiers, key_mods, numpad_char, to_term_key};
use streams::{PtyOutput, pty_stream, watch_stream};
use termherd_core::browser::project_label;
use terminal::{CELL_H, CELL_W, notify, open_url};

/// Sidebar width and the chrome reserved around the terminal, in logical px.
/// Combined with the cell metrics ([`terminal::CELL_W`]/[`CELL_H`]) to size the
/// PTY grid to the window (FR4 resize).
const SIDEBAR_W: f32 = 300.0;
/// Width the collapsed sidebar still occupies (#21): just the slim "▶" handle.
/// The grid reserves this instead of `SIDEBAR_W` when hidden, so the reclaimed
/// space becomes columns rather than stretched cells (#64). The view pins the
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

/// Resolved user configuration handed to the shell at startup: the theme,
/// keymap and metadata overlay built from `settings.json` / `metadata.json`.
/// Bundled so the composition root passes one value, not a long argument list.
pub struct Startup {
    pub theme: ThemeChoice,
    pub keymap: Keymap,
    pub metadata: HashMap<String, SessionMeta>,
    /// Folded project paths restored from disk (#22).
    pub collapsed: HashSet<String>,
}

pub fn run(
    scanner: Arc<dyn ProjectScanner>,
    watch_root: Option<PathBuf>,
    pty: Arc<dyn PtyHost>,
    pty_rx: UnboundedReceiver<PtyEvent>,
    startup: Startup,
) -> iced::Result {
    let config = WindowConfig::load();
    let position = match (config.x, config.y) {
        (Some(x), Some(y)) => window::Position::Specific(Point::new(x, y)),
        _ => window::Position::Centered,
    };
    let pty_output = PtyOutput::new(pty_rx);
    iced::application(
        move || {
            let shell = Shell::new(
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
    /// Browsable plan / memory docs (F-plans-memory), refreshed on scan.
    docs: Vec<DocEntry>,
    /// The doc currently open in the main pane for viewing/editing, if any.
    open_doc: Option<OpenDoc>,
    /// A close awaiting confirmation: the tab index to kill, or `None` (#9).
    /// Killing a session is destructive, so the close button arms this and a
    /// confirmation bar must be accepted before the PTY is actually killed.
    closing: Option<usize>,
    /// An archive awaiting confirmation: the session id to archive, or `None`
    /// (#20). Archiving is easy to trigger by accident, so the archive button
    /// arms this and a confirmation bar must be accepted first. Un-archiving is
    /// harmless and stays a one-click action.
    archiving: Option<String>,
    /// A window close awaiting confirmation: the window id to close once the
    /// user accepts, or `None`. Quitting hard-kills every live session's Claude
    /// process (TerminateProcess / SIGKILL, no graceful shutdown), so a quit
    /// with sessions still running arms this modal first.
    closing_window: Option<window::Id>,
    /// Whether Ctrl (or Cmd) is currently held — the link-open modifier (#28).
    /// Tracked from keyboard events and handed to the terminal canvas so it can
    /// highlight a hovered link and open it on click.
    link_modifier: bool,
}

#[derive(Debug, Clone)]
enum Message {
    Window(window::Id, window::Event),
    ScanCompleted(Result<Vec<SessionRecord>, String>),
    /// The fs watcher saw the projects tree change (FR2).
    ProjectsChanged,
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
    /// A session reported a new title over OSC (#24); relabel its tab.
    PtyTitle {
        session: SessionId,
        title: String,
    },
    /// A session fired an OSC 9 notification (#29); forward it to the OS.
    PtyNotify {
        session: SessionId,
        body: String,
    },
    /// A session's process exited.
    PtyExited(SessionId),
    /// A raw key press; routed to the focused terminal when it has focus.
    Key(keyboard::Event),
    /// IME-composed text (dead/accent keys, CJK) for the focused terminal (#34).
    ImeCommit(String),
    /// Give keyboard focus to the terminal / the search box.
    FocusTerminal,
    FocusSearch,
    /// The mouse wheel scrolled the terminal by a line delta (FR4 scrollback).
    TermScroll(i32),
    /// Copy the given text (a terminal selection) to the clipboard (FR4).
    CopySelection(String),
    /// Clipboard contents read back for a paste into the focused terminal (FR4).
    Paste(Option<String>),
    /// Bring the tab at this index to the front (FR5).
    ActivateTab(usize),
    /// Ask to close the tab at this index — arms the confirmation bar (#9).
    RequestCloseTab(usize),
    /// Confirm the pending close, killing the tab's session(s) (FR5, #9).
    CloseTab(usize),
    /// Dismiss the close confirmation without killing anything (#9).
    CancelClose,
    /// Confirm quitting TermHerd, closing the window (and hard-killing every
    /// live session). Reached only after the quit modal is accepted.
    ConfirmCloseWindow,
    /// Dismiss the quit confirmation, keeping the app and its sessions running.
    CancelCloseWindow,
    /// Toggle a browsed session's star (F-session-metadata).
    ToggleStar(String),
    /// Toggle a browsed session's archived flag (F-session-metadata). Used
    /// directly only to un-archive (a harmless one-click restore); archiving
    /// goes through the confirmation flow below (#20).
    ToggleArchive(String),
    /// Ask to archive a session — arms the confirmation bar (#20).
    RequestArchive(String),
    /// Confirm the pending archive, hiding the session (#20).
    ConfirmArchive,
    /// Dismiss the archive confirmation without archiving (#20).
    CancelArchive,
    /// Show or hide archived sessions in the browser (F-session-metadata).
    ShowArchived(bool),
    /// Fold or unfold a project's session list in the sidebar, by path (#22).
    ToggleCollapsed(String),
    /// Collapse or restore the whole session-browser sidebar (#21).
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
    /// Open a Ctrl/Cmd+clicked terminal link in the OS default handler (#28).
    OpenUrl(String),
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
                | Self::TermScroll(_)
                | Self::Paste(_)
                | Self::ActivateTab(_)
                | Self::RequestCloseTab(_)
                | Self::CloseTab(_)
                | Self::ToggleStar(_)
                | Self::ToggleArchive(_)
                | Self::RequestArchive(_)
                | Self::ToggleCollapsed(_)
                | Self::ToggleSidebar
                | Self::OpenDoc { .. }
                | Self::CloseDoc
        )
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
            docs: crate::docs::discover(&[]),
            open_doc: None,
            closing: None,
            archiving: None,
            closing_window: None,
            link_modifier: false,
        }
    }

    /// The GUI chrome theme (FR10); the terminal grid keeps its own colours.
    fn theme(&self) -> Theme {
        self.theme.clone()
    }

    /// Run one scan off the UI thread (FR2) and feed the result back.
    fn rescan(&self) -> Task<Message> {
        let scanner = self.scanner.clone();
        Task::perform(
            async move { scanner.scan().map_err(|e| e.to_string()) },
            Message::ScanCompleted,
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
                // Fold state is a file write too (#22).
                Effect::SaveCollapsed(collapsed) => {
                    crate::collapsed_store::save(&collapsed);
                    Ok(())
                }
                // Opening a link is an OS handoff, not a PTY call (#28).
                Effect::OpenUrl(url) => open_url(&url),
                // A desktop notification is an OS handoff too (#29).
                Effect::Notify { title, body } => notify(&title, &body),
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
        // same repo stay distinct (#23); it's only the initial label — once the
        // process titles itself over OSC (#24), that wins.
        let label = project_label(&cwd);
        let title = match &launch {
            Launch::Shell => format!("{label} $"),
            Launch::Claude { .. } => format!("{label} 🤖"),
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
        // Opening another session drops any pending confirmation (#9, #20): a
        // stray Enter in the terminal must not confirm a sidebar prompt that's
        // no longer in view.
        self.closing = None;
        self.archiving = None;
        Task::batch([spawn, self.resize_focused()])
    }

    /// Move the focused terminal's viewport (#44): the mouse wheel sends a
    /// relative delta, the scroll-top/bottom shortcuts an absolute jump. Shared
    /// so both paths go through the one `Event::ScrollViewport`.
    fn scroll_focused(&mut self, target: ScrollTarget) -> Task<Message> {
        let Some(session) = self.core.workspace.focused_session() else {
            return Task::none();
        };
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

    /// Collapse or restore the sidebar (#21), then resize the focused terminal
    /// so the grid re-derives its column count for the new width — without this
    /// the cells just stretch to fill the reclaimed space (#64). Shared by the
    /// button (`Message::ToggleSidebar`) and the keymap (`Action::ToggleSidebar`).
    fn toggle_sidebar(&mut self) -> Task<Message> {
        let _ = self.core.apply(termherd_core::Event::ToggleSidebar);
        self.resize_focused()
    }

    /// The terminal grid size (cols, rows) that fits the current window. The
    /// sidebar's width is only reserved while it's visible; collapsing it (#21)
    /// hands that space to the grid as extra columns instead of stretching the
    /// existing cells (#64).
    fn grid_size(&self) -> (u16, u16) {
        let sidebar = if self.core.sidebar_hidden {
            HANDLE_W
        } else {
            SIDEBAR_W
        };
        let avail_w = (self.bounds.width - sidebar - H_CHROME).max(CELL_W);
        let avail_h = (self.bounds.height - V_CHROME).max(CELL_H);
        let cols = (avail_w / CELL_W).floor().clamp(20.0, 500.0) as u16;
        let rows = (avail_h / CELL_H).floor().clamp(5.0, 200.0) as u16;
        (cols, rows)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        // Clicking (or typing) anywhere else in TermHerd while an inline rename
        // is open discards it — the blur-cancels-edit convention. Only genuine
        // user interactions dismiss it; background traffic (PTY output,
        // rescans, window events) and the rename's own messages must not, or a
        // chatty terminal would cancel the edit before it could be typed.
        if self.renaming.is_some() && message.dismisses_rename() {
            self.renaming = None;
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
                // Refresh plan/memory docs now that the project paths are known
                // (a project's CLAUDE.md sits in its real directory).
                let paths: Vec<String> =
                    self.core.projects.iter().map(|g| g.path.clone()).collect();
                self.docs = crate::docs::discover(&paths);
                Task::none()
            }
            Message::ScanCompleted(Err(error)) => {
                tracing::warn!(%error, "scan failed");
                self.scan_error = Some(error);
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
                // reaches the terminal (#28).
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
            Message::TermScroll(delta) => self.scroll_focused(ScrollTarget::Delta(delta)),
            Message::CopySelection(text) => {
                if text.is_empty() {
                    Task::none()
                } else {
                    self.selection = Some(text.clone());
                    iced::clipboard::write(text)
                }
            }
            Message::ActivateTab(index) => self.activate_tab(index),
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
            Message::ConfirmCloseWindow => match self.closing_window.take() {
                Some(id) => window::close(id),
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
                // vanished id would persist phantom metadata for it (#20).
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
    /// the prompt was up (#20).
    fn is_browsable(&self, session: &str) -> bool {
        self.core
            .projects
            .iter()
            .any(|group| group.sessions.iter().any(|s| s.session_id == session))
    }

    /// Arm the confirmation bar for the tab at `index` (#9). No-op for an
    /// out-of-range index, so a stale request can never close the wrong tab.
    fn request_close(&mut self, index: usize) -> Task<Message> {
        if index < self.core.workspace.tabs.len() {
            self.closing = Some(index);
        }
        Task::none()
    }

    /// Close the tab at `index`, killing its session(s) (FR5). Reached only
    /// after the confirmation is accepted (#9): the close button and the
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
    /// drops any pending confirmation (#9, #20). An out-of-range index is a
    /// silent no-op in `core`, so a number key with no matching tab does
    /// nothing (issue #26).
    fn activate_tab(&mut self, index: usize) -> Task<Message> {
        let _ = self.core.apply(termherd_core::Event::ActivateTab(index));
        self.focus = Focus::Terminal;
        self.closing = None;
        self.archiving = None;
        self.resize_focused()
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
            // Number-row jump straight to a tab (issue #26). An index past the
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
        // A pending close confirmation captures the keyboard (#9): Enter
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
        // A pending archive confirmation likewise owns the keyboard (#20):
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
        if self.focus != Focus::Terminal {
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
        let Some(session) = self.core.workspace.focused_session() else {
            return Task::none();
        };
        // A configured shortcut wins over raw terminal input: build the chord
        // and run its action if the keymap binds one (FR9). Unbound keys fall
        // through to the terminal, so plain Ctrl+C stays the interrupt signal.
        if let Some(chord) = chord_of(&key, &physical_key, modifiers)
            && let Some(action) = self.keymap.lookup(&chord)
        {
            return self.run_action(action);
        }
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
    /// step by step, shared so the IME path can't drift from it (#34).
    fn accepts_terminal_input(&self) -> bool {
        self.focus == Focus::Terminal
            && self.renaming.is_none()
            && self.closing.is_none()
            && self.open_doc.is_none()
            && !self.quit_pending()
    }

    /// Route IME-composed text (dead/accent keys, CJK) to the focused terminal
    /// as typed bytes (#34). A commit only fires while the terminal accepts
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
                // Closing the window hard-kills every live session's Claude
                // process. With sessions running, confirm first; otherwise quit
                // straight away. Bounds are already saved either way.
                if self.live_session_count() == 0 {
                    tracing::info!("no live sessions; closing");
                    window::close(id)
                } else {
                    self.closing_window = Some(id);
                    Task::none()
                }
            }
            _ => Task::none(),
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
                metadata: HashMap::new(),
                collapsed: HashSet::new(),
            },
        );
        let _ = shell.launch("/tmp/project".to_string(), Launch::Shell);
        assert!(
            shell.core.workspace.focused_session().is_some(),
            "a launched terminal should be focused"
        );
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
        // tab for the same repo (#23); OSC retitling (#24) takes over later.
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
        // #34: a dead-key composition (e.g. `^` then `e`) reaches the terminal
        // as the resolved character's UTF-8 bytes.
        let (mut shell, pty) = shell_with_terminal();
        let _ = shell.update(Message::ImeCommit("ê".to_string()));
        assert_eq!(pty.writes(), vec!["ê".as_bytes().to_vec()]);
    }

    #[test]
    fn ime_commit_is_ignored_without_terminal_focus() {
        // The composing overlay (search / rename) owns its own input, so a stray
        // commit must not leak into the terminal when it is not focused (#34).
        let (mut shell, pty) = shell_with_terminal();
        shell.focus = Focus::Search;
        let _ = shell.update(Message::ImeCommit("ê".to_string()));
        assert!(pty.writes().is_empty());
    }

    #[test]
    fn ime_commit_does_not_leak_into_an_inline_rename() {
        // Focus stays on the terminal while renaming inline, so a dead-key
        // composition must not reach the PTY — the rename field owns it (#34).
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
        // A close confirmation captures input (#9); an IME commit must not slip
        // past it to the terminal even though focus is still on it (#34).
        let (mut shell, pty) = shell_with_terminal();
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
        let (mut shell, pty) = shell_with_terminal();
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
        let (mut shell, pty) = shell_with_terminal();
        let _ = shell.update(Message::RequestCloseTab(0));
        let _ = shell.update(Message::CancelClose);
        assert_eq!(shell.closing, None);
        assert_eq!(pty.kill_count(), 0);
    }

    #[test]
    fn the_confirmation_owns_the_keyboard() {
        // Escape dismisses the prompt without killing.
        let (mut shell, pty) = shell_with_terminal();
        let _ = shell.update(Message::RequestCloseTab(0));
        let _ = shell.on_key(press(Key::Named(Named::Escape), Modifiers::default(), None));
        assert_eq!(shell.closing, None);
        assert_eq!(pty.kill_count(), 0);

        // Enter confirms; meanwhile a plain key is swallowed, not sent.
        let (mut shell, pty) = shell_with_terminal();
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
    fn collapsing_the_sidebar_widens_the_grid_and_resizes_the_pty() {
        // #64: hiding the sidebar must grow the column count (the reclaimed
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
        // #44: the scroll-top/bottom shortcuts send an absolute jump to the
        // focused session's PTY, through the same path as the mouse wheel.
        let (mut shell, pty) = shell_with_terminal();
        let _ = shell.run_action(Action::ScrollTop);
        let _ = shell.run_action(Action::ScrollBottom);
        // The wheel shares the path and lands a relative delta.
        let _ = shell.update(Message::TermScroll(3));
        assert_eq!(
            pty.scrolls(),
            vec![
                ScrollTarget::Top,
                ScrollTarget::Bottom,
                ScrollTarget::Delta(3)
            ]
        );
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

        // Confirming consumes the pending id (it drives a window::close task).
        shell.closing_window = Some(window::Id::unique());
        let _ = shell.update(Message::ConfirmCloseWindow);
        assert!(
            shell.closing_window.is_none(),
            "confirming consumes the pending window id"
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
        // click folds the project, a second unfolds it (#22).
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
}
