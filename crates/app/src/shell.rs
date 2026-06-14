//! The iced shell — intentionally thin (ARCHITECTURE §8): translate GUI
//! messages into `core` events, perform the returned `core` effects against
//! the adapters, and render `core` state. M1 gave the session browser; M2
//! adds the embedded terminal: a colour grid drawn on a `canvas`, raw
//! keyboard routed to the focused PTY, resize propagation and OSC status.
//! Scrollback and selection are the remaining FR4 items.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::advanced::text::Shaping;
use iced::advanced::widget::{self, operate, operation::focusable};
use iced::futures::channel::mpsc::UnboundedReceiver;
use iced::futures::{SinkExt, Stream, StreamExt};
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Text};
use iced::widget::{
    button, checkbox, column, container, mouse_area, row, scrollable, text, text_input,
};
use iced::{
    Color, Element, Fill, Font, Pixels, Point, Rectangle, Renderer, Size, Subscription, Task,
    Theme, keyboard, mouse, window,
};
use termherd_core::ports::{ProjectScanner, PtyHost};
use termherd_core::workspace::SessionId;
use termherd_core::{
    Action, Effect, KeyChord, Keymap, LaunchSpec, SessionMeta, SessionRecord, SessionStatus, keymap,
};
use termherd_pty::{PtyEvent, Screen};

use crate::settings::ThemeChoice;
use crate::window_config::WindowConfig;

/// Quiet period before a burst of fs events triggers one rescan.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(500);

/// Terminal cell metrics for the monospace grid. Used both to draw and to
/// translate the pane's pixel size into a PTY cell geometry (FR4 resize).
const FONT_SIZE: f32 = 14.0;
const CELL_W: f32 = 8.4;
const CELL_H: f32 = 18.0;
/// Sidebar width and the chrome reserved around the terminal, in logical px.
const SIDEBAR_W: f32 = 300.0;
const H_CHROME: f32 = 40.0;
const V_CHROME: f32 = 84.0;
/// The terminal's default background (matches `termherd_pty`'s default).
const BG: Color = Color::from_rgb(
    0x11 as f32 / 255.0,
    0x13 as f32 / 255.0,
    0x18 as f32 / 255.0,
);

fn search_id() -> widget::Id {
    widget::Id::new("termherd-search")
}

/// Resolved user configuration handed to the shell at startup: the theme,
/// keymap and metadata overlay built from `settings.json` / `metadata.json`.
/// Bundled so the composition root passes one value, not a long argument list.
pub struct Startup {
    pub theme: ThemeChoice,
    pub keymap: Keymap,
    pub metadata: HashMap<String, SessionMeta>,
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
        ..window::Settings::default()
    })
    // Close requests are intercepted so bounds can be saved first.
    .exit_on_close_request(false)
    .subscription(Shell::subscription)
    .run()
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
}

#[derive(Debug, Clone)]
enum Message {
    Window(window::Id, window::Event),
    ScanCompleted(Result<Vec<SessionRecord>, String>),
    /// The fs watcher saw the projects tree change (FR2).
    ProjectsChanged,
    SearchChanged(String),
    SearchTitlesOnly(bool),
    /// Open a fresh shell in the given project directory (FR4).
    LaunchProject(String),
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
    /// A session's process exited.
    PtyExited(SessionId),
    /// A raw key press; routed to the focused terminal when it has focus.
    Key(keyboard::Event),
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
    /// Close the tab at this index, killing its session(s) (FR5).
    CloseTab(usize),
    /// Toggle a browsed session's star (F-session-metadata).
    ToggleStar(String),
    /// Toggle a browsed session's archived flag (F-session-metadata).
    ToggleArchive(String),
    /// Show or hide archived sessions in the browser (F-session-metadata).
    ShowArchived(bool),
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
                Effect::Scroll { session, delta } => self.pty.scroll(session, delta),
                Effect::Kill(session) => self.pty.kill(session),
                // Metadata persistence is a file write, not a PTY call.
                Effect::SaveMetadata(metadata) => {
                    crate::metadata_store::save(&metadata);
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
    fn launch(&mut self, cwd: String, resume: Option<String>) -> Task<Message> {
        let title = project_label(&cwd).to_owned();
        let effects = self
            .core
            .apply(termherd_core::Event::LaunchSession(LaunchSpec {
                cwd: Some(cwd),
                resume,
                title,
            }));
        let spawn = self.perform(effects);
        self.focus = Focus::Terminal;
        Task::batch([spawn, self.resize_focused()])
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

    /// The terminal grid size (cols, rows) that fits the current window.
    fn grid_size(&self) -> (u16, u16) {
        let avail_w = (self.bounds.width - SIDEBAR_W - H_CHROME).max(CELL_W);
        let avail_h = (self.bounds.height - V_CHROME).max(CELL_H);
        let cols = (avail_w / CELL_W).floor().clamp(20.0, 500.0) as u16;
        let rows = (avail_h / CELL_H).floor().clamp(5.0, 200.0) as u16;
        (cols, rows)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Window(id, event) => self.on_window_event(id, event),
            Message::ScanCompleted(Ok(records)) => {
                tracing::info!(sessions = records.len(), "scan completed");
                self.scan_error = None;
                let effects = self
                    .core
                    .apply(termherd_core::Event::ScanCompleted(records));
                debug_assert!(effects.is_empty());
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
            Message::LaunchProject(cwd) => self.launch(cwd, None),
            Message::LaunchSession { cwd, resume } => self.launch(cwd, Some(resume)),
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
            Message::PtyExited(session) => {
                let _ = self.core.apply(termherd_core::Event::PtyExited(session));
                Task::none()
            }
            Message::Key(event) => self.on_key(event),
            Message::FocusTerminal => {
                self.focus = Focus::Terminal;
                Task::none()
            }
            Message::FocusSearch => {
                self.focus = Focus::Search;
                operate(focusable::focus(search_id()))
            }
            Message::TermScroll(delta) => {
                let Some(session) = self.core.workspace.focused_session() else {
                    return Task::none();
                };
                let effects = self
                    .core
                    .apply(termherd_core::Event::TerminalScrolled { session, delta });
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
            Message::ActivateTab(index) => {
                let _ = self.core.apply(termherd_core::Event::ActivateTab(index));
                self.focus = Focus::Terminal;
                self.resize_focused()
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
            Message::CloseTab(index) => self.close_tab(index),
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
            Message::ShowArchived(show) => {
                let _ = self
                    .core
                    .apply(termherd_core::Event::ShowArchivedToggled(show));
                Task::none()
            }
        }
    }

    /// Close the tab at `index`, killing its session(s) (FR5). Shared by the
    /// tab strip's close button and the `CloseFocused` keymap action.
    fn close_tab(&mut self, index: usize) -> Task<Message> {
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

    /// Switch the active tab by `delta`, wrapping around (FR9 `NextTab` /
    /// `PrevTab`). No-op when nothing is open.
    fn cycle_tab(&mut self, delta: i32) -> Task<Message> {
        let count = self.core.workspace.tabs.len();
        if count == 0 {
            return Task::none();
        }
        let next = (self.core.workspace.active as i32 + delta).rem_euclid(count as i32) as usize;
        let _ = self.core.apply(termherd_core::Event::ActivateTab(next));
        self.focus = Focus::Terminal;
        self.resize_focused()
    }

    /// Run a keymap [`Action`] (FR9). Clipboard actions become iced tasks; tab
    /// actions drive `core`. Actions without a surface yet are no-ops.
    fn run_action(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::Copy => self.copy_selection(),
            Action::Paste => iced::clipboard::read().map(Message::Paste),
            Action::NextTab => self.cycle_tab(1),
            Action::PrevTab => self.cycle_tab(-1),
            Action::CloseFocused => self.close_tab(self.core.workspace.active),
            Action::FocusSearch => {
                self.focus = Focus::Search;
                operate(focusable::focus(search_id()))
            }
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
        if self.focus != Focus::Terminal {
            return Task::none();
        }
        let keyboard::Event::KeyPressed {
            key,
            modifiers,
            text,
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
        if let Some(chord) = chord_of(&key, modifiers)
            && let Some(action) = self.keymap.lookup(&chord)
        {
            return self.run_action(action);
        }
        let Some(bytes) = key_to_bytes(&key, modifiers, text.as_deref()) else {
            return Task::none();
        };
        let effects = self
            .core
            .apply(termherd_core::Event::TerminalInput { session, bytes });
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
                tracing::info!("window bounds saved; closing");
                window::close(id)
            }
            _ => Task::none(),
        }
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

    fn view(&self) -> Element<'_, Message> {
        row![self.sidebar(), self.main_pane()].into()
    }

    /// The session browser (FR1 + FR3): search box, then projects by recency.
    /// Clicking a project opens a fresh shell; clicking a session resumes it.
    fn sidebar(&self) -> Element<'_, Message> {
        let mut search = text_input("Rechercher…", &self.core.search)
            .id(search_id())
            .size(12)
            .padding(6);
        if self.focus == Focus::Search {
            search = search.on_input(Message::SearchChanged);
        }
        // Clicking the box hands keyboard focus to it (disabling terminal keys).
        let search = mouse_area(search).on_press(Message::FocusSearch);
        let titles_only = checkbox(self.core.search_titles_only)
            .label("Titres uniquement")
            .on_toggle(Message::SearchTitlesOnly)
            .text_size(11)
            .size(14);
        let show_archived = checkbox(self.core.show_archived)
            .label("Afficher les archivées")
            .on_toggle(Message::ShowArchived)
            .text_size(11)
            .size(14);

        // Live activity, keyed by the Claude session id each terminal resumed,
        // so a browsed row can show its current status (FR8). If the same
        // session is open twice, the most urgent status wins.
        let mut live: HashMap<&str, SessionStatus> = HashMap::new();
        for s in self.core.sessions.values() {
            if let Some(resume) = s.resume.as_deref() {
                live.entry(resume)
                    .and_modify(|cur| {
                        if s.status.urgency() > cur.urgency() {
                            *cur = s.status;
                        }
                    })
                    .or_insert(s.status);
            }
        }

        let visible = self.core.visible_projects();
        let mut list = column![].spacing(16).padding(12);
        if let Some(error) = &self.scan_error {
            list = list.push(text(format!("Scan impossible : {error}")).size(12));
        } else if visible.is_empty() {
            let label = if self.core.search.trim().is_empty() {
                "Aucune session trouvée."
            } else {
                "Aucun résultat."
            };
            list = list.push(text(label).size(12));
        }
        for group in &visible {
            let open = button(text(project_label(&group.path).to_owned()).size(14))
                .on_press(Message::LaunchProject(group.path.clone()))
                .style(button::text)
                .padding(0);
            let mut g = column![open].spacing(4);
            for s in &group.sessions {
                let id = s.session_id.as_str();
                let starred = self.core.is_starred(id);
                let archived = self.core.is_archived(id);

                // Star toggles the pin; archive hides/shows (F-session-metadata).
                let star = button(text(if starred { "★" } else { "☆" }).size(12))
                    .on_press(Message::ToggleStar(s.session_id.clone()))
                    .style(button::text)
                    .padding(0);

                let mut content = row![].spacing(6).align_y(iced::Center);
                // A coloured dot marks a session already open in TermHerd and
                // carries its live activity (FR8).
                if let Some(status) = live.get(id) {
                    content = content.push(text("●").size(9).color(status_style(*status).1));
                }
                let title = self.core.session_title(s);
                content = content.push(
                    text(format!(
                        "{}  ·  {}",
                        clip(&title, 30),
                        s.digest.message_count
                    ))
                    .size(11),
                );
                let launch = button(content)
                    .on_press(Message::LaunchSession {
                        cwd: group.path.clone(),
                        resume: s.session_id.clone(),
                    })
                    .style(button::text)
                    .padding(0)
                    .width(Fill);

                let archive = button(text(if archived { "⊞" } else { "⊟" }).size(12))
                    .on_press(Message::ToggleArchive(s.session_id.clone()))
                    .style(button::text)
                    .padding(0);

                g = g.push(row![star, launch, archive].spacing(6).align_y(iced::Center));
            }
            list = list.push(g);
        }
        container(
            column![
                search,
                titles_only,
                show_archived,
                scrollable(list).height(Fill)
            ]
            .spacing(8)
            .padding(8),
        )
        .width(300)
        .style(container::rounded_box)
        .into()
    }

    /// The focused terminal: a status badge, then its grid drawn on a canvas.
    /// With no session open, a short summary of what the browser found.
    fn main_pane(&self) -> Element<'_, Message> {
        let focused = self.core.workspace.focused_session();
        let screen = focused.and_then(|id| self.screens.get(&id));

        let body: Element<'_, Message> = match screen {
            Some(screen) => {
                let canvas = Canvas::new(TerminalView { screen })
                    .width(Fill)
                    .height(Fill);
                mouse_area(canvas).on_press(Message::FocusTerminal).into()
            }
            None => {
                let total: usize = self.core.projects.iter().map(|g| g.sessions.len()).sum();
                iced::widget::center(
                    column![
                        text("TermHerd").size(40),
                        text(format!(
                            "{} session(s) dans {} projet(s)",
                            total,
                            self.core.projects.len()
                        ))
                        .size(14),
                        text("Cliquez un projet pour ouvrir un terminal,").size(13),
                        text("ou une session pour la reprendre.").size(13),
                    ]
                    .spacing(8)
                    .align_x(iced::Center),
                )
                .height(Fill)
                .into()
            }
        };

        let mut pane = column![].spacing(8).padding(8);
        if let Some(bar) = self.tab_bar() {
            pane = pane.push(bar);
        }
        if let Some(status) = focused.and_then(|id| self.core.sessions.get(&id)) {
            pane = pane.push(status_badge(status.status));
        }
        container(pane.push(body)).width(Fill).height(Fill).into()
    }

    /// The tab strip (FR5): one chip per open session, the active one
    /// highlighted, each carrying its activity dot (FR8) and a close button.
    /// `None` when nothing is open, so the welcome view keeps the full pane.
    fn tab_bar(&self) -> Option<Element<'_, Message>> {
        let tabs = &self.core.workspace.tabs;
        if tabs.is_empty() {
            return None;
        }
        let mut bar = row![].spacing(4).align_y(iced::Center);
        for (index, tab) in tabs.iter().enumerate() {
            let active = index == self.core.workspace.active;
            let mut label = row![].spacing(6).align_y(iced::Center);
            if let Some(status) = self.core.tab_status(index) {
                label = label.push(text("●").size(9).color(status_style(status).1));
            }
            label = label.push(text(clip(&tab.title, 24)).size(12));
            let title = button(label)
                .on_press(Message::ActivateTab(index))
                .padding(6);
            let title = if active {
                title.style(button::primary)
            } else {
                title.style(button::text)
            };
            let close = button(text("×").size(14))
                .on_press(Message::CloseTab(index))
                .style(button::text)
                .padding(4);
            bar = bar.push(row![title, close].align_y(iced::Center));
        }
        Some(bar.into())
    }
}

/// A canvas program that draws the visible terminal grid with per-cell colour
/// and the cursor (FR4), and handles wheel scrollback + drag-to-select.
struct TerminalView<'a> {
    screen: &'a Screen,
}

/// Per-canvas selection state: the drag in progress and the last range.
#[derive(Default)]
struct TermState {
    selecting: bool,
    anchor: Option<(u16, u16)>,
    head: Option<(u16, u16)>,
}

impl canvas::Program<Message> for TerminalView<'_> {
    type State = TermState;

    fn update(
        &self,
        state: &mut TermState,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let canvas::Event::Mouse(event) = event else {
            return None;
        };
        match event {
            // Wheel scrolls the viewport into scrollback history (FR4).
            mouse::Event::WheelScrolled { delta } => {
                let lines = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => y / CELL_H,
                };
                let delta = lines.round() as i32;
                (delta != 0).then(|| canvas::Action::publish(Message::TermScroll(delta)))
            }
            // Drag to select; the press is not captured so the wrapping
            // `mouse_area` still hands keyboard focus to the terminal.
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                cell_at(cursor, bounds, self.screen).map(|cell| {
                    state.selecting = true;
                    state.anchor = Some(cell);
                    state.head = Some(cell);
                    canvas::Action::request_redraw()
                })
            }
            mouse::Event::CursorMoved { .. } if state.selecting => {
                cell_at(cursor, bounds, self.screen).map(|cell| {
                    state.head = Some(cell);
                    canvas::Action::request_redraw()
                })
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) if state.selecting => {
                state.selecting = false;
                match (state.anchor, state.head) {
                    (Some(a), Some(b)) => Some(canvas::Action::publish(Message::CopySelection(
                        selection_text(self.screen, a, b),
                    ))),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &TermState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let cols = self.screen.cols.max(1) as f32;
        let rows = self.screen.rows.max(1) as f32;
        let cell_w = bounds.width / cols;
        let cell_h = bounds.height / rows;

        frame.fill_rectangle(Point::ORIGIN, bounds.size(), BG);

        for (r, line) in self.screen.lines.iter().enumerate() {
            let y = r as f32 * cell_h;
            for (c, cell) in line.iter().enumerate() {
                let x = c as f32 * cell_w;
                if cell.bg != [0x11, 0x13, 0x18] {
                    frame.fill_rectangle(Point::new(x, y), Size::new(cell_w, cell_h), rgb(cell.bg));
                }
                if cell.c != ' ' && cell.c != '\0' {
                    frame.fill_text(Text {
                        content: cell.c.to_string(),
                        position: Point::new(x, y),
                        color: rgb(cell.fg),
                        size: Pixels(FONT_SIZE),
                        font: Font::MONOSPACE,
                        shaping: Shaping::Advanced,
                        ..Text::default()
                    });
                }
            }
        }

        // Translucent overlay over the selected range.
        if let (Some(a), Some(b)) = (state.anchor, state.head) {
            let (start, end) = ordered(a, b);
            for r in start.1..=end.1 {
                let (c0, c1) = selection_span(start, end, r, self.screen.cols);
                let x = c0 as f32 * cell_w;
                let w = (c1.saturating_sub(c0) + 1) as f32 * cell_w;
                frame.fill_rectangle(
                    Point::new(x, r as f32 * cell_h),
                    Size::new(w, cell_h),
                    Color {
                        a: 0.3,
                        ..rgb([0x55, 0x88, 0xff])
                    },
                );
            }
        }

        if let Some((cc, cr)) = self.screen.cursor {
            let x = cc as f32 * cell_w;
            let y = cr as f32 * cell_h;
            frame.fill_rectangle(
                Point::new(x, y),
                Size::new(cell_w, cell_h),
                Color {
                    a: 0.6,
                    ..rgb([0xd0, 0xd0, 0xd0])
                },
            );
        }

        vec![frame.into_geometry()]
    }
}

fn rgb([r, g, b]: [u8; 3]) -> Color {
    Color::from_rgb8(r, g, b)
}

/// The grid cell under the cursor, if any.
fn cell_at(cursor: mouse::Cursor, bounds: Rectangle, screen: &Screen) -> Option<(u16, u16)> {
    let p = cursor.position_in(bounds)?;
    let cols = screen.cols.max(1);
    let rows = screen.rows.max(1);
    let cw = bounds.width / cols as f32;
    let ch = bounds.height / rows as f32;
    if cw <= 0.0 || ch <= 0.0 {
        return None;
    }
    let c = (p.x / cw).floor().clamp(0.0, (cols - 1) as f32) as u16;
    let r = (p.y / ch).floor().clamp(0.0, (rows - 1) as f32) as u16;
    Some((c, r))
}

/// Order two cells in reading order (row, then column).
fn ordered(a: (u16, u16), b: (u16, u16)) -> ((u16, u16), (u16, u16)) {
    if (a.1, a.0) <= (b.1, b.0) {
        (a, b)
    } else {
        (b, a)
    }
}

/// The selected column span `[c0, c1]` on row `r` of an ordered selection.
fn selection_span(start: (u16, u16), end: (u16, u16), r: u16, cols: u16) -> (u16, u16) {
    let last = cols.saturating_sub(1);
    if start.1 == end.1 {
        (start.0.min(end.0), start.0.max(end.0))
    } else if r == start.1 {
        (start.0, last)
    } else if r == end.1 {
        (0, end.0)
    } else {
        (0, last)
    }
}

/// Extract the selected text from the visible grid, trimming trailing blanks.
fn selection_text(screen: &Screen, a: (u16, u16), b: (u16, u16)) -> String {
    let (start, end) = ordered(a, b);
    let mut out = String::new();
    for r in start.1..=end.1 {
        let Some(line) = screen.lines.get(r as usize) else {
            continue;
        };
        let (c0, c1) = selection_span(start, end, r, screen.cols);
        let c0 = c0 as usize;
        let c1 = (c1 as usize).min(line.len().saturating_sub(1));
        if c0 <= c1 {
            let row: String = line[c0..=c1].iter().map(|cell| cell.c).collect();
            out.push_str(row.trim_end());
        }
        if r != end.1 {
            out.push('\n');
        }
    }
    out
}

/// The label and dot colour for an activity status (FR8). Shared by the
/// focused-terminal badge and the sidebar's per-session dot so both stay in
/// sync.
fn status_style(status: SessionStatus) -> (&'static str, Color) {
    match status {
        SessionStatus::Starting => ("démarrage", Color::from_rgb(0.55, 0.55, 0.6)),
        SessionStatus::Busy => ("occupé", Color::from_rgb(0.95, 0.7, 0.2)),
        SessionStatus::Idle => ("prêt", Color::from_rgb(0.3, 0.8, 0.4)),
        SessionStatus::Attention => ("attention", Color::from_rgb(0.95, 0.35, 0.35)),
        SessionStatus::Exited => ("terminé", Color::from_rgb(0.5, 0.5, 0.5)),
    }
}

/// A small per-session activity badge (FR8): a coloured dot + label for the
/// focused terminal. The same dot annotates live rows in the sidebar and each
/// tab in the tab strip.
fn status_badge(status: SessionStatus) -> Element<'static, Message> {
    let (label, color) = status_style(status);
    row![text("●").size(13).color(color), text(label).size(13)]
        .spacing(6)
        .align_y(iced::Center)
        .into()
}

/// The keymap chord for a key press (FR9): the key's normalised name plus the
/// modifier bits. `None` for keys we do not bind (so they reach the terminal).
fn chord_of(key: &keyboard::Key, modifiers: keyboard::Modifiers) -> Option<KeyChord> {
    let name = key_name(key)?;
    let mut mods = 0u8;
    if modifiers.control() {
        mods |= keymap::MOD_CTRL;
    }
    if modifiers.alt() {
        mods |= keymap::MOD_ALT;
    }
    if modifiers.shift() {
        mods |= keymap::MOD_SHIFT;
    }
    if modifiers.logo() {
        mods |= keymap::MOD_CMD;
    }
    Some(KeyChord::new(name, mods))
}

/// The keymap name of an iced key: a lowercased character, or a handful of
/// named keys that bindings use. `None` for keys no shortcut can target.
fn key_name(key: &keyboard::Key) -> Option<String> {
    use keyboard::key::Named;
    match key {
        keyboard::Key::Character(c) => c
            .chars()
            .next()
            .map(|ch| ch.to_ascii_lowercase().to_string()),
        keyboard::Key::Named(Named::Tab) => Some("tab".to_string()),
        keyboard::Key::Named(Named::Enter) => Some("enter".to_string()),
        keyboard::Key::Named(Named::Escape) => Some("escape".to_string()),
        _ => None,
    }
}

/// Translate a key press into the bytes a terminal expects (FR4): control
/// combinations, the common named keys and cursor sequences, otherwise the
/// layout-resolved text.
fn key_to_bytes(
    key: &keyboard::Key,
    modifiers: keyboard::Modifiers,
    text: Option<&str>,
) -> Option<Vec<u8>> {
    use keyboard::Key;
    use keyboard::key::Named;

    if modifiers.control()
        && let Key::Character(c) = key
        && let Some(ch) = c.chars().next()
    {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphabetic() {
            return Some(vec![(lower as u8 - b'a') + 1]);
        }
        match ch {
            ' ' => return Some(vec![0]),
            '[' => return Some(vec![27]),
            '\\' => return Some(vec![28]),
            ']' => return Some(vec![29]),
            _ => {}
        }
    }

    match key {
        Key::Named(named) => {
            let seq: &[u8] = match named {
                Named::Enter => b"\r",
                Named::Backspace => b"\x7f",
                Named::Tab => b"\t",
                Named::Escape => b"\x1b",
                Named::ArrowUp => b"\x1b[A",
                Named::ArrowDown => b"\x1b[B",
                Named::ArrowRight => b"\x1b[C",
                Named::ArrowLeft => b"\x1b[D",
                Named::Home => b"\x1b[H",
                Named::End => b"\x1b[F",
                Named::Delete => b"\x1b[3~",
                Named::PageUp => b"\x1b[5~",
                Named::PageDown => b"\x1b[6~",
                Named::Space => b" ",
                _ => return None,
            };
            Some(seq.to_vec())
        }
        Key::Character(_) | Key::Unidentified => text
            .filter(|t| !t.is_empty())
            .map(|t| t.as_bytes().to_vec()),
    }
}

/// Streams PTY output/exit into the subscription. Wraps the channel receiver
/// so it can be moved into the stream once; the `Arc` identity makes the
/// subscription stable across `view`/`update` cycles (it hashes by pointer).
#[derive(Clone)]
struct PtyOutput(Arc<Mutex<Option<UnboundedReceiver<PtyEvent>>>>);

impl PtyOutput {
    fn new(rx: UnboundedReceiver<PtyEvent>) -> Self {
        Self(Arc::new(Mutex::new(Some(rx))))
    }
}

impl Hash for PtyOutput {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.0) as usize).hash(state);
    }
}

/// One PTY-output stream: drains the receiver into [`Message`]s. The receiver
/// is taken on first run; a duplicated subscription (there is only ever one)
/// would idle forever rather than steal events.
fn pty_stream(output: &PtyOutput) -> impl Stream<Item = Message> + use<> {
    let taken = output.0.lock().ok().and_then(|mut slot| slot.take());
    iced::stream::channel(
        64,
        |mut out: iced::futures::channel::mpsc::Sender<Message>| async move {
            match taken {
                Some(mut rx) => {
                    while let Some(event) = rx.next().await {
                        let message = match event {
                            PtyEvent::Output { session, screen } => {
                                Message::PtyOutput { session, screen }
                            }
                            PtyEvent::Status { session, status } => {
                                Message::PtyStatus { session, status }
                            }
                            PtyEvent::Exited { session } => Message::PtyExited(session),
                        };
                        if out.send(message).await.is_err() {
                            break;
                        }
                    }
                }
                None => iced::futures::future::pending::<()>().await,
            }
        },
    )
}

/// One fs-watch stream per projects root: forwards each debounced change
/// burst as a [`Message::ProjectsChanged`]. The watcher lives as long as
/// the stream; if it cannot start, the sidebar simply stops live-updating
/// (logged, not fatal).
// `&PathBuf` is imposed by `Subscription::run_with`, which passes `&D` to a
// plain fn pointer — `&Path` would not match `for<'a> fn(&'a D)`.
#[allow(clippy::ptr_arg)]
fn watch_stream(root: &PathBuf) -> impl Stream<Item = Message> + use<> {
    let root = root.clone();
    iced::stream::channel(
        4,
        |mut output: iced::futures::channel::mpsc::Sender<Message>| async move {
            let (tx, mut rx) = iced::futures::channel::mpsc::unbounded::<()>();
            match termherd_scan::watch_changes(root, WATCH_DEBOUNCE, move || {
                let _ = tx.unbounded_send(());
            }) {
                Ok(handle) => {
                    while rx.next().await.is_some() {
                        if output.send(Message::ProjectsChanged).await.is_err() {
                            break;
                        }
                    }
                    drop(handle);
                }
                Err(error) => {
                    tracing::warn!(%error, "fs watch unavailable; sidebar will not live-update");
                    iced::futures::future::pending::<()>().await;
                }
            }
        },
    )
}

/// Last path component — what the sidebar shows as the project name.
fn project_label(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path)
}

fn clip(s: &str, max: usize) -> String {
    let cleaned: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        let mut out: String = cleaned.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::key::Named;
    use iced::keyboard::{Key, Modifiers};

    fn ctrl() -> Modifiers {
        Modifiers::CTRL
    }

    #[test]
    fn control_letters_map_to_control_bytes() {
        // Ctrl-C -> 0x03, Ctrl-A -> 0x01.
        assert_eq!(
            key_to_bytes(&Key::Character("c".into()), ctrl(), Some("c")),
            Some(vec![3])
        );
        assert_eq!(
            key_to_bytes(&Key::Character("a".into()), ctrl(), Some("a")),
            Some(vec![1])
        );
    }

    #[test]
    fn named_keys_map_to_their_sequences() {
        let none = Modifiers::default();
        assert_eq!(
            key_to_bytes(&Key::Named(Named::Enter), none, None),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            key_to_bytes(&Key::Named(Named::ArrowUp), none, None),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key_to_bytes(&Key::Named(Named::Backspace), none, None),
            Some(b"\x7f".to_vec())
        );
    }

    #[test]
    fn characters_send_their_resolved_text() {
        let none = Modifiers::default();
        assert_eq!(
            key_to_bytes(&Key::Character("é".into()), none, Some("é")),
            Some("é".as_bytes().to_vec())
        );
        // No text and not a known named key -> nothing to send.
        assert_eq!(key_to_bytes(&Key::Unidentified, none, None), None);
    }

    #[test]
    fn chord_of_builds_keymap_chords_from_key_events() {
        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
        assert_eq!(
            chord_of(&Key::Character("C".into()), ctrl_shift),
            Some(KeyChord::new("c", keymap::MOD_CTRL | keymap::MOD_SHIFT))
        );
        assert_eq!(
            chord_of(&Key::Named(Named::Tab), Modifiers::CTRL),
            Some(KeyChord::new("tab", keymap::MOD_CTRL))
        );
        // Keys no shortcut targets carry no chord.
        assert_eq!(chord_of(&Key::Named(Named::F2), Modifiers::default()), None);
    }
}
