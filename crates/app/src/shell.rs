//! The iced shell — intentionally thin (ARCHITECTURE §8): translate GUI
//! messages into `core` events, perform the returned `core` effects against
//! the adapters, and render `core` state. M1 gave the session browser; M2
//! adds the embedded terminal (minimal slice: live screen text + a line of
//! input). Colours, raw key input, scrollback and tabs/splits come next.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::futures::channel::mpsc::UnboundedReceiver;
use iced::futures::{SinkExt, Stream, StreamExt};
use iced::widget::{button, checkbox, column, container, row, scrollable, text, text_input};
use iced::{Element, Fill, Font, Point, Size, Subscription, Task, window};
use termherd_core::ports::ProjectScanner;
use termherd_core::ports::PtyHost;
use termherd_core::workspace::SessionId;
use termherd_core::{Effect, LaunchSpec, SessionRecord};
use termherd_pty::PtyEvent;

use crate::window_config::WindowConfig;

/// Quiet period before a burst of fs events triggers one rescan.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(500);

pub fn run(
    scanner: Arc<dyn ProjectScanner>,
    watch_root: Option<PathBuf>,
    pty: Arc<dyn PtyHost>,
    pty_rx: UnboundedReceiver<PtyEvent>,
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
            );
            let initial_scan = shell.rescan();
            (shell, initial_scan)
        },
        Shell::update,
        Shell::view,
    )
    .title(|_: &Shell| String::from("TermHerd"))
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
    /// Latest rendered screen text per session (minimal slice).
    screens: HashMap<SessionId, String>,
    /// The command-line input box for the focused terminal.
    input_line: String,
}

#[derive(Debug, Clone)]
enum Message {
    Window(window::Id, window::Event),
    ScanCompleted(Result<Vec<SessionRecord>, String>),
    /// The fs watcher saw the projects tree change (FR2).
    ProjectsChanged,
    SearchChanged(String),
    SearchTitlesOnly(bool),
    /// Launch a terminal in the given project directory (FR4).
    LaunchProject(String),
    /// New screen contents for a session.
    PtyOutput {
        session: SessionId,
        screen: String,
    },
    /// A session's process exited.
    PtyExited(SessionId),
    /// The command-line input box changed.
    InputLineChanged(String),
    /// Send the current input line to the focused terminal.
    InputLineSubmit,
}

impl Shell {
    fn new(
        bounds: WindowConfig,
        scanner: Arc<dyn ProjectScanner>,
        watch_root: Option<PathBuf>,
        pty: Arc<dyn PtyHost>,
        pty_output: PtyOutput,
    ) -> Self {
        Self {
            core: termherd_core::App::new(),
            bounds,
            scanner,
            watch_root,
            scan_error: None,
            pty,
            pty_output,
            screens: HashMap::new(),
            input_line: String::new(),
        }
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
                Effect::Kill(session) => self.pty.kill(session),
            };
            if let Err(error) = outcome {
                tracing::warn!(%error, "pty effect failed");
            }
        }
        Task::none()
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
            Message::LaunchProject(cwd) => {
                let title = project_label(&cwd).to_owned();
                let effects = self
                    .core
                    .apply(termherd_core::Event::LaunchSession(LaunchSpec {
                        cwd: Some(cwd),
                        resume: None,
                        title,
                    }));
                self.perform(effects)
            }
            Message::PtyOutput { session, screen } => {
                self.screens.insert(session, screen);
                Task::none()
            }
            Message::PtyExited(session) => {
                let _ = self.core.apply(termherd_core::Event::PtyExited(session));
                if let Some(screen) = self.screens.get_mut(&session) {
                    screen.push_str("\n\n[processus terminé]");
                }
                Task::none()
            }
            Message::InputLineChanged(value) => {
                self.input_line = value;
                Task::none()
            }
            Message::InputLineSubmit => {
                let Some(session) = self.core.workspace.focused_session() else {
                    return Task::none();
                };
                let mut bytes = std::mem::take(&mut self.input_line).into_bytes();
                bytes.push(b'\r');
                let effects = self
                    .core
                    .apply(termherd_core::Event::TerminalInput { session, bytes });
                self.perform(effects)
            }
        }
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
                Task::none()
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
        let mut subs = vec![window::events().map(|(id, event)| Message::Window(id, event))];
        if let Some(root) = &self.watch_root {
            subs.push(Subscription::run_with(root.clone(), watch_stream));
        }
        subs.push(Subscription::run_with(self.pty_output.clone(), pty_stream));
        Subscription::batch(subs)
    }

    fn view(&self) -> Element<'_, Message> {
        row![self.sidebar(), self.main_pane()].into()
    }

    /// The session browser (FR1 + FR3): search box, then projects by
    /// recency. Clicking a project opens a terminal in it (FR4).
    fn sidebar(&self) -> Element<'_, Message> {
        let search = text_input("Rechercher…", &self.core.search)
            .on_input(Message::SearchChanged)
            .size(12)
            .padding(6);
        let titles_only = checkbox(self.core.search_titles_only)
            .label("Titres uniquement")
            .on_toggle(Message::SearchTitlesOnly)
            .text_size(11)
            .size(14);

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
                g = g.push(
                    text(format!(
                        "{}  ·  {}",
                        clip(s.digest.display_title(None), 36),
                        s.digest.message_count
                    ))
                    .size(11),
                );
            }
            list = list.push(g);
        }
        container(
            column![search, titles_only, scrollable(list).height(Fill)]
                .spacing(8)
                .padding(8),
        )
        .width(300)
        .style(container::rounded_box)
        .into()
    }

    /// The focused terminal: its live screen text plus a line of input. With
    /// no session open, a short summary of what the browser found.
    fn main_pane(&self) -> Element<'_, Message> {
        let focused = self.core.workspace.focused_session();
        let screen = focused.and_then(|id| self.screens.get(&id));

        let body: Element<'_, Message> = match screen {
            Some(text_content) => {
                scrollable(text(text_content.clone()).font(Font::MONOSPACE).size(13))
                    .height(Fill)
                    .width(Fill)
                    .into()
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
                        text("Cliquez un projet pour ouvrir un terminal.").size(13),
                    ]
                    .spacing(8)
                    .align_x(iced::Center),
                )
                .height(Fill)
                .into()
            }
        };

        let input = text_input("commande…  (Entrée pour envoyer)", &self.input_line)
            .on_input(Message::InputLineChanged)
            .on_submit(Message::InputLineSubmit)
            .font(Font::MONOSPACE)
            .padding(8);

        container(column![body, input].spacing(8).padding(8))
            .width(Fill)
            .height(Fill)
            .into()
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
