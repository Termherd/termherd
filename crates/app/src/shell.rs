//! The iced shell — intentionally thin (ARCHITECTURE §8): translate GUI
//! messages into `core` events, perform returned effects, render `core`
//! state. M1: real session browser in the sidebar; terminals land in M2.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use iced::futures::{SinkExt, Stream, StreamExt};
use iced::widget::{checkbox, column, container, row, scrollable, text, text_input};
use iced::{Element, Fill, Point, Size, Subscription, Task, window};
use termherd_core::SessionRecord;
use termherd_core::ports::ProjectScanner;

use crate::window_config::WindowConfig;

/// Quiet period before a burst of fs events triggers one rescan.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(500);

pub fn run(scanner: Arc<dyn ProjectScanner>, watch_root: Option<PathBuf>) -> iced::Result {
    let config = WindowConfig::load();
    let position = match (config.x, config.y) {
        (Some(x), Some(y)) => window::Position::Specific(Point::new(x, y)),
        _ => window::Position::Centered,
    };
    iced::application(
        move || {
            let shell = Shell::new(config, scanner.clone(), watch_root.clone());
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
    /// The headless core; all browser state lives there.
    core: termherd_core::App,
    bounds: WindowConfig,
    scanner: Arc<dyn ProjectScanner>,
    watch_root: Option<PathBuf>,
    scan_error: Option<String>,
}

#[derive(Debug, Clone)]
enum Message {
    Window(window::Id, window::Event),
    ScanCompleted(Result<Vec<SessionRecord>, String>),
    /// The fs watcher saw the projects tree change (FR2).
    ProjectsChanged,
    SearchChanged(String),
    SearchTitlesOnly(bool),
}

impl Shell {
    fn new(
        bounds: WindowConfig,
        scanner: Arc<dyn ProjectScanner>,
        watch_root: Option<PathBuf>,
    ) -> Self {
        Self {
            core: termherd_core::App::new(),
            bounds,
            scanner,
            watch_root,
            scan_error: None,
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
        Subscription::batch(subs)
    }

    fn view(&self) -> Element<'_, Message> {
        row![self.sidebar(), self.main_pane()].into()
    }

    /// The session browser (FR1 + FR3): search box, then projects by
    /// recency with their matching sessions.
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
            let mut g = column![text(project_label(&group.path).to_owned()).size(14)].spacing(4);
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

    fn main_pane(&self) -> Element<'_, Message> {
        let total: usize = self.core.projects.iter().map(|g| g.sessions.len()).sum();
        iced::widget::center(
            column![
                text("TermHerd").size(40),
                text(format!(
                    "{} session(s) dans {} projet(s) — terminaux en M2",
                    total,
                    self.core.projects.len()
                ))
                .size(14),
            ]
            .spacing(8)
            .align_x(iced::Center),
        )
        .into()
    }
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
