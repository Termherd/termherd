//! Confirmation modals and the scrim that hosts them. Quit, tab-close and
//! archive all confirm through the one centred [`modal`] overlay (so the three
//! flows look and behave alike), driven by [`Shell::active_confirmation`] which
//! picks the single armed confirmation in priority order.

use iced::widget::{button, center, container, mouse_area, opaque, row, stack, text};
use iced::{Color, Element};

use super::clip;
use crate::shell::{Message, Shell};
use crate::strings;

impl Shell {
    /// The active confirmation card and the message to fire when its backdrop is
    /// dismissed. Quit, tab-close and archive are all routed through the same
    /// [`modal`] presentation for parity, in priority order (quit > tab-close >
    /// archive — at most one is armed at a time); `view` wraps the base UI with
    /// it. `pub(in crate::shell)` so the shell's own tests can assert the
    /// routing.
    pub(in crate::shell) fn active_confirmation(&self) -> Option<(Element<'_, Message>, Message)> {
        self.quit_confirmation()
            .map(|card| (card, Message::CancelCloseWindow))
            .or_else(|| {
                self.close_confirmation()
                    .map(|card| (card, Message::CancelClose))
            })
            .or_else(|| {
                self.archive_confirmation()
                    .map(|card| (card, Message::CancelArchive))
            })
    }

    /// The quit-confirmation card (shown when a window close is armed and live
    /// sessions would be hard-killed). `None` when no quit is pending.
    fn quit_confirmation(&self) -> Option<Element<'_, Message>> {
        if !self.quit_pending() {
            return None;
        }
        let live = self.live_session_count();
        Some(Self::confirmation_bar(
            strings::quit_prompt(live),
            strings::QUIT,
            button::danger,
            Message::ConfirmCloseWindow,
            Message::CancelCloseWindow,
        ))
    }

    /// The close-confirmation card, shown when a tab close is armed: it names
    /// the session about to die and offers Fermer (confirm) / Annuler. `None`
    /// when nothing is pending or the armed index has since gone away.
    fn close_confirmation(&self) -> Option<Element<'_, Message>> {
        let index = self.closing?;
        let tab = self.core.workspace.tabs.get(index)?;
        Some(Self::confirmation_bar(
            strings::close_tab_prompt(&clip(&tab.title, 24)),
            strings::CLOSE,
            button::danger,
            Message::CloseTab(index),
            Message::CancelClose,
        ))
    }

    /// The archive-confirmation card, shown when archiving a session is armed:
    /// it names the session and offers Archiver (confirm) / Annuler. `None` when
    /// nothing is pending.
    fn archive_confirmation(&self) -> Option<Element<'_, Message>> {
        let session = self.archiving.as_deref()?;
        let title = self
            .core
            .projects
            .iter()
            .flat_map(|group| &group.sessions)
            .find(|s| s.session_id == session)
            .map_or_else(|| session.to_owned(), |s| self.core.session_title(s));
        Some(Self::confirmation_bar(
            strings::archive_prompt(&clip(&title, 24)),
            strings::ARCHIVE,
            button::primary,
            Message::ConfirmArchive,
            Message::CancelArchive,
        ))
    }

    /// A confirmation card shared by the quit, close and archive prompts:
    /// the question, a styled confirm button, and an Annuler cancel, in the
    /// rounded container they use. Keeping one builder stops the prompts
    /// drifting apart.
    fn confirmation_bar<'a>(
        prompt: String,
        confirm_label: &'a str,
        confirm_style: impl Fn(&iced::Theme, button::Status) -> button::Style + 'a,
        on_confirm: Message,
        on_cancel: Message,
    ) -> Element<'a, Message> {
        let prompt = text(prompt).size(12);
        let confirm = button(text(confirm_label).size(12))
            .on_press(on_confirm)
            .style(confirm_style)
            .padding(6);
        let cancel = button(text(strings::CANCEL).size(12))
            .on_press(on_cancel)
            .style(button::text)
            .padding(6);
        container(
            row![prompt, confirm, cancel]
                .spacing(12)
                .align_y(iced::Center),
        )
        .padding(6)
        .style(container::rounded_box)
        .into()
    }
}

/// Overlay `content` as a centred modal over `base`, dimming everything behind
/// it; a click on the scrim emits `on_blur` to dismiss. The base UI keeps
/// rendering underneath but cannot be interacted with — the inner `opaque`
/// swallows clicks on the card, the outer one blocks the layers below.
pub(super) fn modal<'a>(
    base: Element<'a, Message>,
    content: Element<'a, Message>,
    on_blur: Message,
) -> Element<'a, Message> {
    stack(vec![
        base,
        opaque(mouse_area(center(opaque(content)).style(modal_backdrop)).on_press(on_blur)),
    ])
    .into()
}

/// Semi-transparent scrim drawn behind a modal so the dialog reads as the only
/// actionable surface.
fn modal_backdrop(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(
            Color {
                a: 0.6,
                ..Color::BLACK
            }
            .into(),
        ),
        ..container::Style::default()
    }
}
