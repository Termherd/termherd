//! The plan / memory document pane (F-plans-memory): a header (label, save
//! state, actions) above an editable text area. When a doc is open it takes
//! over the main pane; a read-only doc omits the Save control. No state
//! transitions live here — those are in the parent module.

use iced::theme::palette::Extended;
use iced::widget::{button, column, container, row, text, text_editor};
use iced::{Color, Element, Fill, Font};

use super::super::Message;
use super::super::docs::{DocFeedback, OpenDoc};
use super::style::{ATTENTION_HUE, IDLE_HUE, text_on_surface};
#[cfg(test)]
use super::style::{TEXT_FLOOR, contrast};
use crate::strings;

/// Render an open plan/memory doc: a header (label, save state, actions) above
/// the editable text area (F-plans-memory). A read-only doc omits the Save
/// control; the header always carries a close button. `editable` is off while
/// any prompt sits over the editor: a modal blocks only the mouse, and a
/// focused editor would otherwise take the very Enter that answers it.
pub(super) fn doc_editor(doc: &OpenDoc, editable: bool) -> Element<'_, Message> {
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

    let mut editor = text_editor(&doc.content)
        .font(Font::MONOSPACE)
        .size(12)
        .height(Fill);
    if editable {
        editor = editor.on_action(Message::DocEdit);
    }

    container(column![header, editor].spacing(8).padding(8))
        .width(Fill)
        .height(Fill)
        .into()
}

/// The save-state note for the editor header: the last save outcome if there is
/// one, else a "modified" hint while there are unsaved edits, else nothing.
fn save_note(doc: &OpenDoc) -> Option<Element<'_, Message>> {
    let (note, label) = match &doc.feedback {
        Some(DocFeedback::Saved) => (Note::Saved, strings::DOC_SAVED.to_owned()),
        Some(DocFeedback::Error(message)) => (Note::Error, message.clone()),
        None if doc.dirty => (Note::Modified, strings::DOC_MODIFIED.to_owned()),
        None => return None,
    };
    Some(
        text(label)
            .size(11)
            .style(move |theme: &iced::Theme| text::Style {
                color: Some(note_color(note, theme.extended_palette())),
            })
            .into(),
    )
}

/// What the save note says, which decides its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Note {
    Saved,
    Error,
    Modified,
}

#[cfg(test)]
impl Note {
    const ALL: [Self; 3] = [Self::Saved, Self::Error, Self::Modified];
}

fn note_color(note: Note, palette: &Extended) -> Color {
    let hue = match note {
        Note::Saved => IDLE_HUE,
        Note::Error => ATTENTION_HUE,
        Note::Modified => Color::from_rgb(0.6, 0.6, 0.6),
    };
    text_on_surface(hue, palette)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ThemeChoice;

    #[test]
    fn every_save_note_reads_as_text_on_every_theme() {
        for theme in ThemeChoice::ALL.map(ThemeChoice::to_iced) {
            let palette = theme.extended_palette();
            let surface = palette.background.base.color;
            for note in Note::ALL {
                let ratio = contrast(note_color(note, palette), surface);
                assert!(ratio >= TEXT_FLOOR, "{note:?} on {theme}: {ratio:.2}");
            }
        }
    }

    #[test]
    fn the_saved_note_stays_green_and_the_error_red() {
        for theme in ThemeChoice::ALL.map(ThemeChoice::to_iced) {
            let palette = theme.extended_palette();
            let saved = note_color(Note::Saved, palette);
            let error = note_color(Note::Error, palette);
            assert!(
                saved.g > saved.r && saved.g > saved.b,
                "saved on {theme}: {saved:?}"
            );
            assert!(
                error.r > error.g && error.r > error.b,
                "error on {theme}: {error:?}"
            );
        }
    }
}
