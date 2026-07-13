//! The plan / memory document pane (F-plans-memory): a header (label, save
//! state, actions) above an editable text area. When a doc is open it takes
//! over the main pane; a read-only doc omits the Save control. No state
//! transitions live here — those are in the parent module.

use iced::widget::{button, column, container, row, text, text_editor};
use iced::{Color, Element, Fill, Font};

use super::super::{DocFeedback, Message, OpenDoc};
use crate::strings;

/// Render an open plan/memory doc: a header (label, save state, actions) above
/// the editable text area (F-plans-memory). A read-only doc omits the Save
/// control; the header always carries a close button.
pub(super) fn doc_editor(doc: &OpenDoc) -> Element<'_, Message> {
    let mut header = row![text(&doc.label).size(13)]
        .spacing(12)
        .align_y(iced::Center);

    if doc.writable {
        // A disabled button (no `on_press`) reads as "nothing to save".
        let mut save = button(text(strings::DOC_SAVE).size(12))
            .style(button::text)
            .padding(0);
        if doc.dirty {
            save = save.on_press(Message::SaveDoc);
        }
        header = header.push(save);
    }

    if let Some(note) = save_note(doc) {
        header = header.push(note);
    }

    header = header.push(
        button(text(strings::DOC_CLOSE).size(12))
            .on_press(Message::CloseDoc)
            .style(button::text)
            .padding(0),
    );

    let editor = text_editor(&doc.content)
        .on_action(Message::DocEdit)
        .font(Font::MONOSPACE)
        .size(12)
        .height(Fill);

    container(column![header, editor].spacing(8).padding(8))
        .width(Fill)
        .height(Fill)
        .into()
}

/// The save-state note for the editor header: the last save outcome if there is
/// one, else a "modified" hint while there are unsaved edits, else nothing.
fn save_note(doc: &OpenDoc) -> Option<Element<'_, Message>> {
    const SAVED: Color = Color::from_rgb(0.3, 0.8, 0.4);
    const ERROR: Color = Color::from_rgb(0.95, 0.35, 0.35);
    const DIRTY: Color = Color::from_rgb(0.6, 0.6, 0.6);

    match &doc.feedback {
        Some(DocFeedback::Saved) => Some(text(strings::DOC_SAVED).size(11).color(SAVED).into()),
        Some(DocFeedback::Error(message)) => {
            Some(text(message.clone()).size(11).color(ERROR).into())
        }
        None if doc.dirty => Some(text(strings::DOC_MODIFIED).size(11).color(DIRTY).into()),
        None => None,
    }
}
