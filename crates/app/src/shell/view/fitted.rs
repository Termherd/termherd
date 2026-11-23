//! One line of text cut to the width the layout gives it, with an ellipsis
//! where it runs out: the sidebar's titles, labels and matched lines.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Tree, Widget, tree};
use iced::advanced::{mouse, renderer};
use iced::widget::text;
use iced::{Element, Length, Rectangle, Renderer, Size, Theme};

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
/// it, by [`fit`]. It fills the width it is given unless told otherwise.
pub(super) fn fitted<'a, M>(
    s: String,
    size: f32,
    style: fn(&Theme) -> text::Style,
) -> Fitted<'a, M> {
    Fitted {
        source: s,
        size,
        style,
        width: Length::Fill,
        max_width: f32::INFINITY,
        reserve: 0.0,
        content: Element::new(iced::widget::space()),
    }
}

/// A line of text cut at layout, when its width is known. The cut is kept in
/// the widget's state and remade only when the text, size or width change.
pub(super) struct Fitted<'a, M> {
    source: String,
    size: f32,
    style: fn(&Theme) -> text::Style,
    width: Length,
    max_width: f32,
    reserve: f32,
    /// The text widget drawing the cut, rebuilt at each layout.
    content: Element<'a, M>,
}

impl<M> Fitted<'_, M> {
    /// How wide the line may grow: `Shrink` takes only what its text needs.
    pub(super) fn width(mut self, width: Length) -> Self {
        self.width = width;
        self
    }

    /// The widest the line may grow, in pixels.
    pub(super) fn max_width(mut self, max_width: f32) -> Self {
        self.max_width = max_width;
        self
    }

    /// Room to leave, in pixels, out of the width the line is given, for
    /// what follows it in its row: a `Shrink` line laid out first would
    /// otherwise take all of it.
    pub(super) fn reserve(mut self, reserve: f32) -> Self {
        self.reserve = reserve;
        self
    }
}

impl<M> Widget<M, Theme, Renderer> for Fitted<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<FitCache>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(FitCache::default())
    }

    fn diff(&self, _tree: &mut Tree) {
        // The text child is built at layout, so it is diffed there.
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.max_width(self.max_width);
        let limits = limits
            .max_width((limits.max().width - self.reserve).max(0.0))
            .width(self.width)
            .height(Length::Shrink);
        let width = limits.max().width;
        let (source, size) = (&self.source, self.size);
        let line = tree
            .state
            .downcast_mut::<FitCache>()
            .get(source, size, width, || fit(source, size, width))
            .to_owned();
        self.content = text(line)
            .size(size)
            .wrapping(text::Wrapping::None)
            .style(self.style)
            .into();
        tree.diff_children(std::slice::from_ref(&self.content));
        let node =
            self.content
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, &limits.loose());
        let size = limits.resolve(self.width, Length::Shrink, node.size());
        layout::Node::with_children(size, vec![node])
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if let (Some(child), Some(child_layout)) = (tree.children.first(), layout.children().next())
        {
            self.content.as_widget().draw(
                child,
                renderer,
                theme,
                style,
                child_layout,
                cursor,
                viewport,
            );
        }
    }
}

impl<'a, M: 'a> From<Fitted<'a, M>> for Element<'a, M> {
    fn from(fitted: Fitted<'a, M>) -> Self {
        Element::new(fitted)
    }
}

/// The last cut a fitted text made, kept across layouts so that a scroll or
/// a redraw at an unchanged width measures nothing.
#[derive(Debug, Default)]
struct FitCache {
    key: Option<(String, u32, u32)>,
    fitted: String,
}

impl FitCache {
    /// The cut of `source` at `size` within `width`: the one kept when the
    /// three are those of the last call, else what `cut` computes.
    fn get(&mut self, source: &str, size: f32, width: f32, cut: impl FnOnce() -> String) -> &str {
        let fresh = self
            .key
            .as_ref()
            .is_some_and(|(s, z, w)| s == source && *z == size.to_bits() && *w == width.to_bits());
        if !fresh {
            self.fitted = cut();
            self.key = Some((source.to_owned(), size.to_bits(), width.to_bits()));
        }
        &self.fitted
    }
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
    fn a_relayout_with_nothing_changed_measures_nothing() {
        let mut cache = FitCache::default();
        let mut cuts = 0;
        let mut layout = |source: &str, size: f32, width: f32| {
            cache
                .get(source, size, width, || {
                    cuts += 1;
                    format!("{source}@{width}")
                })
                .to_owned()
        };
        for _ in 0..3 {
            assert_eq!(layout("fix the race", 11.0, 100.0), "fix the race@100");
        }
        layout("fix the race", 11.0, 90.0);
        layout("fix the cache", 11.0, 90.0);
        layout("fix the cache", 10.0, 90.0);
        assert_eq!(cuts, 4, "one cut per change of text, size or width");
    }

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
