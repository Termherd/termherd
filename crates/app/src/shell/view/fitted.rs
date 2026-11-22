//! One line of text cut to the width the layout gives it, with an ellipsis
//! where it runs out: the sidebar's titles, labels and matched lines.

use iced::Element;
use iced::widget::text;

/// `s` on one line, cut to the longest start that fits `max_width` pixels at
/// `size` with an ellipsis after it, measured in the font the sidebar draws.
pub(super) fn fit(s: &str, size: f32, max_width: f32) -> String {
    let line = s.replace('\n', " ");
    if text_width(&line, size) <= max_width {
        return line;
    }
    let with_ellipsis =
        |kept: usize| -> String { line.chars().take(kept).chain(['…']).collect() };
    // Width grows with every character kept, so the longest start that fits
    // is found by halving: `lo` always fits, `hi` never does.
    let (mut lo, mut hi) = (0, line.chars().count());
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if text_width(&with_ellipsis(mid), size) <= max_width {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    with_ellipsis(lo)
}

/// `s` as one line of text cut to the width its place in the layout gives
/// it, by [`fit`]. The cut is made at layout, when that width is known.
pub(super) fn fitted<'a, M: 'a>(
    s: String,
    size: f32,
    style: fn(&iced::Theme) -> text::Style,
) -> Element<'a, M> {
    iced::widget::responsive(move |space| {
        text(fit(&s, size, space.width))
            .size(size)
            .wrapping(text::Wrapping::None)
            .style(style)
            .into()
    })
    .height(iced::Length::Shrink)
    .into()
}

/// The width `s` takes on one line at `size`, in the default font.
fn text_width(s: &str, size: f32) -> f32 {
    use iced::advanced::text::Paragraph as _;
    let paragraph = iced::advanced::graphics::text::Paragraph::with_text(iced::advanced::Text {
        content: s,
        bounds: iced::Size::INFINITE,
        size: size.into(),
        line_height: text::LineHeight::default(),
        font: iced::Font::DEFAULT,
        align_x: text::Alignment::Default,
        align_y: iced::alignment::Vertical::Top,
        shaping: text::Shaping::default(),
        wrapping: text::Wrapping::None,
    });
    paragraph.min_bounds().width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_that_fits_is_kept_whole_on_one_line() {
        assert_eq!(fit("fix the\nrace", 11.0, 1000.0), "fix the race");
    }

    #[test]
    fn a_long_line_keeps_the_longest_start_that_fits_then_an_ellipsis() {
        let line = "fix the race in the session watcher before the release";
        assert!(text_width("abc", 11.0) > 0.0, "the font measures nothing");
        let max = text_width(line, 11.0) / 2.0;
        let fitted = fit(line, 11.0, max);
        assert!(fitted.ends_with('…'), "{fitted:?}");
        assert!(text_width(&fitted, 11.0) <= max, "{fitted:?} overflows");
        let kept = fitted.trim_end_matches('…').chars().count();
        let one_more: String = line.chars().take(kept + 1).chain(['…']).collect();
        assert!(text_width(&one_more, 11.0) > max, "{one_more:?} also fits");
    }

    #[test]
    fn the_cut_follows_glyph_width_not_character_count() {
        let max = text_width("iiiiiiiiii", 11.0);
        let wide = fit(&"W".repeat(40), 11.0, max);
        let narrow = fit(&"i".repeat(40), 11.0, max);
        assert!(
            wide.chars().count() < narrow.chars().count(),
            "{wide} / {narrow}"
        );
    }

    #[test]
    fn a_space_too_narrow_for_any_letter_still_shows_the_ellipsis() {
        assert_eq!(fit("fix the race", 11.0, 1.0), "…");
    }
}
