//! The settings panel: a card over the workspace whose picks apply live — the
//! chrome on the next frame, every terminal through the appearance sink — and
//! save themselves. No state transitions live here; those are in
//! `shell::appearance`.

use iced::widget::{button, column, container, pick_list, row, text};
use iced::{Element, Fill};

use super::sidebar_secondary_text;
use crate::settings::ThemeChoice;
use crate::shell::appearance::SchemeChoice;
use crate::shell::{Message, Shell};
use crate::strings;

impl Shell {
    pub(super) fn settings_panel(&self) -> Element<'_, Message> {
        let header = row![
            text(strings::SETTINGS_TITLE).size(16),
            iced::widget::Space::new().width(Fill),
            button(text(strings::SETTINGS_CLOSE).size(12))
                .on_press(Message::CloseSettings)
                .style(button::text)
                .padding(4),
        ]
        .align_y(iced::Center);

        let theme = pick_list(
            ThemeChoice::ALL,
            Some(self.appearance.theme),
            Message::PickTheme,
        )
        .text_size(12)
        .width(180);
        let scheme = pick_list(
            SchemeChoice::all(),
            SchemeChoice::of(self.appearance.colors.scheme.as_deref()),
            Message::PickScheme,
        )
        .text_size(12)
        .width(180);

        let field = |label: &'static str, control: Element<'static, Message>| {
            row![text(label).size(12).width(Fill), control]
                .spacing(12)
                .align_y(iced::Center)
        };

        container(
            column![
                header,
                field(strings::SETTINGS_THEME, theme.into()),
                field(strings::SETTINGS_SCHEME, scheme.into()),
                text(strings::SETTINGS_SAVED_NOTE)
                    .size(11)
                    .style(sidebar_secondary_text),
                text(strings::SETTINGS_CLAUDE_NOTE)
                    .size(11)
                    .style(sidebar_secondary_text),
            ]
            .spacing(12),
        )
        .padding(16)
        .width(420)
        .style(container::rounded_box)
        .into()
    }
}
