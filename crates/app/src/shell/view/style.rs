//! Palette-derived visual styling shared across the view: the activity-dot
//! colour, the hover-card surface and its text tiers, the sidebar's muted
//! secondary text, plus the small `mix`/`clip` primitives they build on. Every
//! colour is pulled from the theme palette rather than hardcoded, so the whole
//! view tracks the theme system once it lands.

use iced::Color;
use iced::widget::text::Text;
use iced::widget::{container, text};
use termherd_core::{ClaudeColor, SessionKind, SessionStatus};

/// The mark for what a session runs, shared by the sidebar's launch buttons
/// and the tab chips so the button that opens a tab shows what the tab will
/// show: `✳` is the glyph Claude Code puts in its own title, `❯` a shell
/// prompt.
pub(super) fn kind_glyph(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Claude => "✳",
        SessionKind::Shell => "❯",
    }
}

/// The tab chip's kind mark. It takes the chip's text colour, leaving colour
/// to the status dot beside it.
pub(super) fn kind_icon<'a>(kind: SessionKind) -> Text<'a> {
    text(kind_glyph(kind)).size(12)
}

/// The activity dot for a status (FR8). Shared by the tab strip's chips and
/// the sidebar's per-session dots so both stay in sync. The colour is the only
/// place the UI shows a status; the word form an agent reads lives in
/// [`crate::snapshot_dto`], not on the screen.
pub(super) fn status_dot<'a>(status: SessionStatus) -> Text<'a> {
    text("●")
        .size(9)
        .style(move |theme: &iced::Theme| text::Style {
            color: Some(status_color(status, theme.extended_palette().is_dark)),
        })
}

/// The dot colour for a status. The hues are tuned against a dark surface;
/// on a light one they wash out, so they are darkened by the same step to keep
/// the dot legible without changing which hue means what.
fn status_color(status: SessionStatus, dark_surface: bool) -> Color {
    let hue = match status {
        SessionStatus::Starting => Color::from_rgb(0.55, 0.55, 0.6),
        SessionStatus::Busy => Color::from_rgb(0.95, 0.7, 0.2),
        SessionStatus::Idle => Color::from_rgb(0.3, 0.8, 0.4),
        SessionStatus::Attention => Color::from_rgb(0.95, 0.35, 0.35),
        SessionStatus::Exited => Color::from_rgb(0.5, 0.5, 0.5),
    };
    on_surface(hue, dark_surface)
}

/// A hue tuned against a dark surface, darkened on a light one so it keeps
/// its contrast without changing which hue means what.
fn on_surface(hue: Color, dark_surface: bool) -> Color {
    if dark_surface {
        hue
    } else {
        mix(hue, Color::BLACK, LIGHT_SURFACE_DARKEN)
    }
}

/// How far a status hue moves toward black on a light surface.
const LIGHT_SURFACE_DARKEN: f32 = 0.4;

/// What a `/color` name paints in termherd's chrome, or `None` for
/// [`ClaudeColor::Default`], which leaves a session with no colour of its own.
/// Like the status dots, the hues are tuned against a dark surface and
/// darkened by the same step on a light one.
pub(super) fn claude_color(color: ClaudeColor, dark_surface: bool) -> Option<Color> {
    let hue = match color {
        ClaudeColor::Red => Color::from_rgb(1.0, 0.42, 0.40),
        ClaudeColor::Blue => Color::from_rgb(0.42, 0.62, 1.0),
        ClaudeColor::Green => Color::from_rgb(0.36, 0.78, 0.38),
        ClaudeColor::Yellow => Color::from_rgb(0.78, 0.70, 0.06),
        ClaudeColor::Purple => Color::from_rgb(0.74, 0.52, 1.0),
        ClaudeColor::Orange => Color::from_rgb(0.98, 0.58, 0.15),
        ClaudeColor::Pink => Color::from_rgb(0.95, 0.45, 0.80),
        ClaudeColor::Cyan => Color::from_rgb(0.24, 0.80, 0.86),
        ClaudeColor::Default => return None,
    };
    Some(on_surface(hue, dark_surface))
}

/// The width of a session colour's mark: a chip's outline, a sidebar bar.
pub(super) const COLOR_MARK_WIDTH: f32 = 2.0;

/// The sidebar row's colour mark: a thin bar in the colour `/color` set, or
/// `None` when the session has none. A bar rather than a tinted title, so the
/// title keeps the contrast the theme gave it.
pub(super) fn color_bar<'a, M: 'a>(color: ClaudeColor) -> Option<container::Container<'a, M>> {
    if color == ClaudeColor::Default {
        return None;
    }
    Some(
        container(text(""))
            .width(COLOR_MARK_WIDTH + 1.0)
            .height(14)
            .style(move |theme: &iced::Theme| container::Style {
                background: claude_color(color, theme.extended_palette().is_dark)
                    .map(iced::Background::Color),
                border: iced::Border {
                    radius: 1.0.into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            }),
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance, enough to compare contrast ratios.
    fn luminance(c: Color) -> f32 {
        let lin = |v: f32| {
            if v <= 0.039_28 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    fn contrast(a: Color, b: Color) -> f32 {
        let (hi, lo) = {
            let (la, lb) = (luminance(a), luminance(b));
            if la > lb { (la, lb) } else { (lb, la) }
        };
        (hi + 0.05) / (lo + 0.05)
    }

    const ALL: [SessionStatus; 5] = [
        SessionStatus::Starting,
        SessionStatus::Busy,
        SessionStatus::Idle,
        SessionStatus::Attention,
        SessionStatus::Exited,
    ];

    #[test]
    fn every_status_dot_reads_on_every_light_chrome() {
        // The WCAG floor for non-text UI components.
        const FLOOR: f32 = 3.0;
        let light = crate::settings::ThemeChoice::ALL
            .map(crate::settings::ThemeChoice::to_iced)
            .into_iter()
            .filter(|theme| !theme.extended_palette().is_dark);
        for theme in light {
            let palette = theme.extended_palette();
            for surface in [palette.background.base.color, palette.background.weak.color] {
                for status in ALL {
                    let ratio = contrast(status_color(status, false), surface);
                    assert!(ratio >= FLOOR, "{status:?} on {theme}: {ratio:.2}");
                }
            }
        }
    }

    /// The colours that paint: every `/color` name but `default`.
    fn painted_colors() -> Vec<ClaudeColor> {
        ClaudeColor::ALL
            .into_iter()
            .filter(|c| *c != ClaudeColor::Default)
            .collect()
    }

    #[test]
    fn default_paints_nothing_and_every_other_name_paints() {
        assert_eq!(claude_color(ClaudeColor::Default, true), None);
        assert_eq!(claude_color(ClaudeColor::Default, false), None);
        let painted = ClaudeColor::ALL
            .into_iter()
            .filter(|c| claude_color(*c, true).is_some())
            .collect::<Vec<_>>();
        assert_eq!(painted, painted_colors());
    }

    #[test]
    fn every_session_colour_reads_on_every_theme_chrome() {
        // The WCAG floor for non-text UI components, against the surfaces a
        // chip outline and a sidebar bar sit on.
        const FLOOR: f32 = 3.0;
        for theme in crate::settings::ThemeChoice::ALL.map(crate::settings::ThemeChoice::to_iced) {
            let palette = theme.extended_palette();
            for surface in [palette.background.base.color, palette.background.weak.color] {
                for color in painted_colors() {
                    let paint = claude_color(color, palette.is_dark).expect("painted");
                    let ratio = contrast(paint, surface);
                    assert!(ratio >= FLOOR, "{color:?} on {theme}: {ratio:.2}");
                }
            }
        }
    }

    #[test]
    fn no_two_session_colours_paint_alike() {
        for dark in [true, false] {
            let palette = painted_colors();
            for (i, a) in palette.iter().enumerate() {
                for b in &palette[i + 1..] {
                    let (pa, pb) = (
                        claude_color(*a, dark).expect("painted"),
                        claude_color(*b, dark).expect("painted"),
                    );
                    let distance = (pa.r - pb.r).abs() + (pa.g - pb.g).abs() + (pa.b - pb.b).abs();
                    assert!(distance > 0.2, "{a:?} and {b:?} (dark: {dark})");
                }
            }
        }
    }

    #[test]
    fn a_dark_surface_keeps_the_original_hues() {
        for status in ALL {
            let dark = status_color(status, true);
            let light = status_color(status, false);
            assert!(luminance(light) < luminance(dark), "{status:?} darkens");
        }
    }
}
