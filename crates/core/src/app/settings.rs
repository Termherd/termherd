//! Terminal font-size settings: the configured base, zoom steps, and the
//! clamped effective size.

use super::*;

/// A zoom request, carried by [`Event::Zoom`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    /// Grow the terminal font one step.
    In,
    /// Shrink the terminal font one step.
    Out,
    /// Back to the configured base size.
    Reset,
}

/// The terminal font size before settings load or when none is configured.
/// Mirrors the historical `FONT_SIZE` constant.
pub const DEFAULT_FONT_SIZE: f32 = 14.0;
/// Bounds for the effective font size — small enough to overview a
/// large scrollback, large enough for a presentation, and both far from
/// degenerate cell geometry.
const FONT_SIZE_RANGE: (f32, f32) = (6.0, 40.0);

/// Terminal font sizing kept on [`App`]: the configured base and the zoom
/// steps applied on top. Grouped so the two travel together rather than as
/// loose fields; the effective size is read through [`App::font_size`].
#[derive(Debug, Default)]
pub(super) struct FontState {
    /// The configured base size, from settings via [`Event::FontSizeLoaded`];
    /// `None` until loaded (the built-in [`DEFAULT_FONT_SIZE`] then applies).
    base: Option<f32>,
    /// Zoom steps on top of the base: ±1 px each, clamped at event time so
    /// surplus presses at a bound don't accumulate as drift. Ephemeral —
    /// resets each launch.
    steps: i32,
}

impl App {
    /// Record the configured terminal base font size, from settings.
    pub(super) fn load_font_size(&mut self, size: f32) -> Vec<Effect> {
        self.font.base = Some(size);
        Vec::new()
    }

    /// The effective terminal font size: the configured base (or the
    /// built-in default before settings load) plus the zoom steps, clamped
    /// into `FONT_SIZE_RANGE`.
    #[must_use]
    pub fn font_size(&self) -> f32 {
        let base = self.font.base.unwrap_or(DEFAULT_FONT_SIZE);
        let (min, max) = FONT_SIZE_RANGE;
        (base + self.font.steps as f32).clamp(min, max)
    }

    /// Apply a zoom step. Steps are refused at the bounds rather than
    /// clamped at read, so surplus presses never accumulate as drift — one
    /// zoom-out after many zoom-ins at the cap shrinks immediately.
    pub(super) fn zoom(&mut self, zoom: Zoom) -> Vec<Effect> {
        let (min, max) = FONT_SIZE_RANGE;
        match zoom {
            Zoom::In if self.font_size() < max => self.font.steps += 1,
            Zoom::Out if self.font_size() > min => self.font.steps -= 1,
            Zoom::Reset => self.font.steps = 0,
            Zoom::In | Zoom::Out => {}
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps_the_font_from_the_loaded_base_and_resets() {
        let mut app = App::new();
        // Before settings load, the built-in default applies.
        assert!((app.font_size() - DEFAULT_FONT_SIZE).abs() < f32::EPSILON);

        app.apply(Event::FontSizeLoaded(16.0));
        assert!((app.font_size() - 16.0).abs() < f32::EPSILON);

        app.apply(Event::Zoom(Zoom::In));
        app.apply(Event::Zoom(Zoom::In));
        assert!((app.font_size() - 18.0).abs() < f32::EPSILON);

        app.apply(Event::Zoom(Zoom::Out));
        assert!((app.font_size() - 17.0).abs() < f32::EPSILON);

        let effects = app.apply(Event::Zoom(Zoom::Reset));
        assert!(effects.is_empty());
        assert!((app.font_size() - 16.0).abs() < f32::EPSILON);
    }

    #[test]
    fn zoom_refuses_steps_at_the_bounds_without_accumulating_drift() {
        let mut app = App::new();
        app.apply(Event::FontSizeLoaded(38.0));
        // Two steps reach the 40.0 cap; ten more must be refused, not banked.
        for _ in 0..12 {
            app.apply(Event::Zoom(Zoom::In));
        }
        assert!((app.font_size() - 40.0).abs() < f32::EPSILON);
        // One zoom-out shrinks immediately — no surplus presses to unwind.
        app.apply(Event::Zoom(Zoom::Out));
        assert!((app.font_size() - 39.0).abs() < f32::EPSILON);

        // Same at the floor.
        app.apply(Event::FontSizeLoaded(7.0));
        app.apply(Event::Zoom(Zoom::Reset));
        for _ in 0..12 {
            app.apply(Event::Zoom(Zoom::Out));
        }
        assert!((app.font_size() - 6.0).abs() < f32::EPSILON);
        app.apply(Event::Zoom(Zoom::In));
        assert!((app.font_size() - 7.0).abs() < f32::EPSILON);
    }
}
