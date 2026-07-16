//! Palette-derived visual styling shared across the view: the activity-dot
//! colour, the hover-card surface and its text tiers, the sidebar's muted
//! secondary text, plus the small `mix`/`clip` primitives they build on. Every
//! colour is pulled from the theme palette rather than hardcoded, so the whole
//! view tracks the theme system once it lands.

use iced::Color;
use iced::widget::container;
use termherd_core::SessionStatus;

/// The dot colour for an activity status (FR8). Shared by the tab strip's
/// chips and the sidebar's per-session dots so both stay in sync. The colour is
/// the only place the UI shows a status; the word form
/// ([`crate::strings::status_label`]) is for the capture dump, not the screen.
pub(super) fn status_color(status: SessionStatus) -> Color {
    match status {
        SessionStatus::Starting => Color::from_rgb(0.55, 0.55, 0.6),
        SessionStatus::Busy => Color::from_rgb(0.95, 0.7, 0.2),
        SessionStatus::Idle => Color::from_rgb(0.3, 0.8, 0.4),
        SessionStatus::Attention => Color::from_rgb(0.95, 0.35, 0.35),
        SessionStatus::Exited => Color::from_rgb(0.5, 0.5, 0.5),
    }
}

/// Background for the session hover card — a step away from the surrounding
/// surface (the `strong` palette tier rather than the default `weak`) so the
/// card reads as a distinct floating layer, with a thin border to seal it.
/// Everything is pulled from the theme palette, so it tracks the theme system
/// once that lands rather than baking in a colour.
pub(super) fn card_style(theme: &iced::Theme) -> container::Style {
    let surface = card_surface(theme);
    container::Style {
        background: Some(surface.color.into()),
        text_color: Some(surface.text),
        border: iced::Border {
            color: theme.extended_palette().background.weak.color,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    }
}

pub(super) fn card_secondary_text(theme: &iced::Theme) -> iced::widget::text::Style {
    let surface = card_surface(theme);
    iced::widget::text::Style {
        color: Some(mix(surface.text, surface.color, 0.35)),
    }
}

/// The palette tier the hover card paints on — its surface colour and the text
/// colour meant to sit on it. Single-sourced so the "which tier" choice (and
/// the eventual theme-system wiring) lives in one place.
fn card_surface(theme: &iced::Theme) -> iced::theme::palette::Pair {
    theme.extended_palette().background.strong
}

/// Dimmed secondary text for the sidebar — search-match snippets. Mixes
/// the normal text toward the background so it reads muted, theme-aware rather
/// than a hardcoded grey.
pub(super) fn sidebar_secondary_text(theme: &iced::Theme) -> iced::widget::text::Style {
    let palette = theme.extended_palette();
    iced::widget::text::Style {
        color: Some(mix(
            palette.background.base.text,
            palette.background.base.color,
            0.4,
        )),
    }
}

/// Linear blend from `a` to `b` by `t` in `[0, 1]`.
pub(super) fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

/// Collapse newlines to spaces and truncate to `max` characters with an ellipsis.
pub(super) fn clip(s: &str, max: usize) -> String {
    let cleaned: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        let mut out: String = cleaned.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
