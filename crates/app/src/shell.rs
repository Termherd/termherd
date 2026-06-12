//! The iced shell — intentionally thin (ARCHITECTURE §8): translate GUI
//! messages into `core` events, perform returned effects, render `core`
//! state. M0 renders a placeholder; the session browser lands in M1.

use iced::widget::{center, column, text};
use iced::{Element, Point, Size, Subscription, Task, window};

use crate::window_config::WindowConfig;

pub fn run() -> iced::Result {
    let config = WindowConfig::load();
    let position = match (config.x, config.y) {
        (Some(x), Some(y)) => window::Position::Specific(Point::new(x, y)),
        _ => window::Position::Centered,
    };
    iced::application(move || Shell::new(config), Shell::update, Shell::view)
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
    /// The headless core. M0 holds it; M1 starts feeding it events.
    core: termherd_core::App,
    bounds: WindowConfig,
}

#[derive(Debug, Clone)]
enum Message {
    Window(window::Id, window::Event),
}

impl Shell {
    fn new(bounds: WindowConfig) -> Self {
        Self {
            core: termherd_core::App::new(),
            bounds,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Window(id, event) => self.on_window_event(id, event),
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
        // M1: render the session browser from `self.core` state.
        let _ = &self.core;
        center(
            column![
                text("TermHerd").size(40),
                text("M0 shell — session browser lands in M1").size(14),
            ]
            .spacing(8)
            .align_x(iced::Center),
        )
        .into()
    }
}
