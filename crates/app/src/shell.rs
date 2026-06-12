//! The iced shell — intentionally thin (ARCHITECTURE §8): translate GUI
//! messages into `core` events, perform returned effects, render `core`
//! state. M1: real session browser in the sidebar; terminals land in M2.

use std::sync::Arc;

use iced::widget::{column, container, row, scrollable, text};
use iced::{Element, Fill, Point, Size, Subscription, Task, window};
use termherd_core::SessionRecord;
use termherd_core::ports::ProjectScanner;

use crate::window_config::WindowConfig;

pub fn run(scanner: Arc<dyn ProjectScanner>) -> iced::Result {
    let config = WindowConfig::load();
    let position = match (config.x, config.y) {
        (Some(x), Some(y)) => window::Position::Specific(Point::new(x, y)),
        _ => window::Position::Centered,
    };
    iced::application(
        move || {
            let shell = Shell::new(config, scanner.clone());
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
    scan_error: Option<String>,
}

#[derive(Debug, Clone)]
enum Message {
    Window(window::Id, window::Event),
    ScanCompleted(Result<Vec<SessionRecord>, String>),
}

impl Shell {
    fn new(bounds: WindowConfig, scanner: Arc<dyn ProjectScanner>) -> Self {
        Self {
            core: termherd_core::App::new(),
            bounds,
            scanner,
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
        window::events().map(|(id, event)| Message::Window(id, event))
    }

    fn view(&self) -> Element<'_, Message> {
        row![self.sidebar(), self.main_pane()].into()
    }

    /// The session browser (FR1): projects by recency, sessions inside.
    fn sidebar(&self) -> Element<'_, Message> {
        let mut list = column![].spacing(16).padding(12);
        if let Some(error) = &self.scan_error {
            list = list.push(text(format!("Scan impossible : {error}")).size(12));
        } else if self.core.projects.is_empty() {
            list = list.push(text("Aucune session trouvée.").size(12));
        }
        for group in &self.core.projects {
            let mut g = column![text(project_label(&group.path)).size(14)].spacing(4);
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
        container(scrollable(list).height(Fill))
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
