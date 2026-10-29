//! Palette-derived visual styling shared across the view: the activity-dot
//! colour, the hover-card surface and its text tiers, the sidebar's muted
//! secondary text, plus the small `mix`/`clip` primitives they build on. Every
//! colour is pulled from the theme palette rather than hardcoded, so the whole
//! view tracks the theme system once it lands.

use iced::theme::palette::Extended;
use iced::widget::text::Text;
use iced::widget::{container, text};
use iced::{Color, Element};
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
            color: Some(status_color(status, theme.extended_palette())),
        })
}

fn status_color(status: SessionStatus, palette: &Extended) -> Color {
    let hue = match status {
        SessionStatus::Starting => Color::from_rgb(0.55, 0.55, 0.6),
        SessionStatus::Busy => Color::from_rgb(0.95, 0.7, 0.2),
        SessionStatus::Idle => Color::from_rgb(0.3, 0.8, 0.4),
        SessionStatus::Attention => Color::from_rgb(0.95, 0.35, 0.35),
        SessionStatus::Exited => Color::from_rgb(0.5, 0.5, 0.5),
    };
    on_surface(hue, palette)
}

/// The hues are tuned for a dark surface. The fixed light-theme darkening
/// suits the stock themes; a mid-tone surface (Nord, Tokyo Night Light) also
/// needs the push away from it, until the mark clears [`MARK_FLOOR`] on the
/// [`mark_surfaces`].
fn on_surface(hue: Color, palette: &Extended) -> Color {
    let (start, away) = if palette.is_dark {
        (hue, Color::WHITE)
    } else {
        (mix(hue, Color::BLACK, LIGHT_SURFACE_DARKEN), Color::BLACK)
    };
    let surfaces = mark_surfaces(palette).map(luminance);
    (0..=SHADE_STEPS)
        .map(|step| mix(start, away, f32::from(step) / f32::from(SHADE_STEPS)))
        .find(|shade| {
            let shade = luminance(*shade);
            surfaces
                .iter()
                .all(|s| contrast_ratio(shade, *s) >= MARK_FLOOR)
        })
        .unwrap_or(away)
}

/// The backgrounds a mark must read on. Not the active tab chip's fill: its
/// status dot is unshaded against it.
fn mark_surfaces(palette: &Extended) -> [Color; 2] {
    [palette.background.base.color, palette.background.weak.color]
}

/// How far a status hue moves toward black on a light surface.
const LIGHT_SURFACE_DARKEN: f32 = 0.4;

/// The WCAG contrast floor for non-text UI components.
const MARK_FLOOR: f32 = 3.0;

/// How finely [`on_surface`] shades a hue toward black or white.
const SHADE_STEPS: u8 = 20;

/// WCAG relative luminance.
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

/// WCAG contrast ratio of two luminances, from 1 to 21.
fn contrast_ratio(la: f32, lb: f32) -> f32 {
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// What a `/color` name paints in termherd's chrome, or `None` for
/// [`ClaudeColor::Default`], which leaves a session with no colour of its own.
pub(super) fn claude_color(color: ClaudeColor, palette: &Extended) -> Option<Color> {
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
    Some(on_surface(hue, palette))
}

/// The width of a session colour's mark: a chip's outline, a sidebar bar.
pub(super) const COLOR_MARK_WIDTH: f32 = 2.0;

/// A square of `color` as the chrome paints it; for `default`, an empty frame,
/// the absence of a colour it stands for.
pub(super) fn color_swatch<'a, M: 'a>(color: ClaudeColor) -> Element<'a, M> {
    container(text(""))
        .width(12)
        .height(12)
        .style(move |theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: claude_color(color, palette).map(iced::Background::Color),
                border: iced::Border {
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}

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
                background: claude_color(color, theme.extended_palette())
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
    use crate::settings::ThemeChoice;

    const ALL: [SessionStatus; 5] = [
        SessionStatus::Starting,
        SessionStatus::Busy,
        SessionStatus::Idle,
        SessionStatus::Attention,
        SessionStatus::Exited,
    ];

    fn every_theme() -> impl Iterator<Item = iced::Theme> {
        ThemeChoice::ALL.map(ThemeChoice::to_iced).into_iter()
    }

    fn contrast(a: Color, b: Color) -> f32 {
        contrast_ratio(luminance(a), luminance(b))
    }

    /// The colours that paint: every `/color` name but `default`.
    fn painted_colors() -> Vec<ClaudeColor> {
        ClaudeColor::ALL
            .into_iter()
            .filter(|c| *c != ClaudeColor::Default)
            .collect()
    }

    #[test]
    fn every_status_dot_reads_on_every_theme_chrome() {
        for theme in every_theme() {
            let palette = theme.extended_palette();
            for surface in mark_surfaces(palette) {
                for status in ALL {
                    let ratio = contrast(status_color(status, palette), surface);
                    assert!(ratio >= MARK_FLOOR, "{status:?} on {theme}: {ratio:.2}");
                }
            }
        }
    }

    #[test]
    fn every_session_colour_reads_on_every_theme_chrome() {
        for theme in every_theme() {
            let palette = theme.extended_palette();
            for surface in mark_surfaces(palette) {
                for color in painted_colors() {
                    let paint = claude_color(color, palette).expect("painted");
                    let ratio = contrast(paint, surface);
                    assert!(ratio >= MARK_FLOOR, "{color:?} on {theme}: {ratio:.2}");
                }
            }
        }
    }

    /// The stock themes paint what they did before the push existed.
    #[test]
    fn the_stock_themes_keep_their_tuned_colours() {
        let stock = [
            ThemeChoice::Dark,
            ThemeChoice::Light,
            ThemeChoice::SolarizedDark,
            ThemeChoice::SolarizedLight,
            ThemeChoice::GruvboxDark,
            ThemeChoice::GruvboxLight,
        ];
        let hues = [
            Color::from_rgb(0.95, 0.7, 0.2),
            Color::from_rgb(1.0, 0.42, 0.40),
        ];
        for theme in stock.map(ThemeChoice::to_iced) {
            let palette = theme.extended_palette();
            for hue in hues {
                let tuned = if palette.is_dark {
                    hue
                } else {
                    mix(hue, Color::BLACK, LIGHT_SURFACE_DARKEN)
                };
                assert_eq!(on_surface(hue, palette), tuned, "{theme}");
            }
        }
    }

    #[test]
    fn a_mid_tone_surface_shades_the_hue_until_it_reads() {
        let palette = iced::Theme::TokyoNightLight.extended_palette();
        let busy = Color::from_rgb(0.95, 0.7, 0.2);
        let stock = mix(busy, Color::BLACK, LIGHT_SURFACE_DARKEN);
        assert!(
            mark_surfaces(palette)
                .iter()
                .any(|s| contrast(stock, *s) < MARK_FLOOR),
            "the stock shade must fail here, or this test proves nothing"
        );
        let shaded = on_surface(busy, palette);
        assert!(luminance(shaded) < luminance(stock), "pushed toward black");
        for surface in mark_surfaces(palette) {
            assert!(contrast(shaded, surface) >= MARK_FLOOR);
        }
    }

    #[test]
    fn default_paints_nothing_and_every_other_name_paints() {
        for theme in [iced::Theme::Dark, iced::Theme::Light] {
            let palette = theme.extended_palette();
            assert_eq!(claude_color(ClaudeColor::Default, palette), None);
            let painted = ClaudeColor::ALL
                .into_iter()
                .filter(|c| claude_color(*c, palette).is_some())
                .collect::<Vec<_>>();
            assert_eq!(painted, painted_colors());
        }
    }

    #[test]
    fn no_two_session_colours_paint_alike() {
        for theme in every_theme() {
            let palette = theme.extended_palette();
            let colors = painted_colors();
            for (i, a) in colors.iter().enumerate() {
                for b in &colors[i + 1..] {
                    let (pa, pb) = (
                        claude_color(*a, palette).expect("painted"),
                        claude_color(*b, palette).expect("painted"),
                    );
                    let distance = (pa.r - pb.r).abs() + (pa.g - pb.g).abs() + (pa.b - pb.b).abs();
                    assert!(distance > 0.2, "{a:?} and {b:?} on {theme}");
                }
            }
        }
    }

    #[test]
    fn a_light_surface_darkens_every_status_hue() {
        let (dark, light) = (
            iced::Theme::Dark.extended_palette(),
            iced::Theme::Light.extended_palette(),
        );
        for status in ALL {
            let on_dark = status_color(status, dark);
            let on_light = status_color(status, light);
            assert!(
                luminance(on_light) < luminance(on_dark),
                "{status:?} darkens"
            );
        }
    }
}
