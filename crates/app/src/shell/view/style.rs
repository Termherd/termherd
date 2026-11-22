//! Palette-derived visual styling shared across the view: the activity-dot
//! colour, the hover-card surface and its text tiers, the sidebar's muted
//! secondary text, plus the small `mix`/`clip` primitives they build on. Every
//! colour is pulled from the theme palette rather than hardcoded, so the whole
//! view tracks the theme system once it lands.

use iced::theme::palette::Extended;
use iced::widget::text::Text;
use iced::widget::{container, text};
use iced::{Color, Element};
use termherd_core::{ClaudeColor, Provider, SessionKind, SessionStatus};

/// The mark for what a session runs, shared by the sidebar's launch buttons
/// and the tab chips so the button that opens a tab shows what the tab will
/// show: `✳` is the glyph Claude Code puts in its own title, `❯` a shell
/// prompt, `◈` Codex, and `✦` Antigravity.
pub(super) fn kind_glyph(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Shell => "❯",
        SessionKind::Agent(Provider::Claude) => "✳",
        SessionKind::Agent(Provider::Codex) => "◈",
        SessionKind::Agent(Provider::Antigravity) => "✦",
    }
}

/// The tab chip's kind mark. It takes the chip's text colour, leaving colour
/// to the status dot beside it.
pub(super) fn kind_icon<'a>(kind: SessionKind) -> Text<'a> {
    text(kind_glyph(kind)).size(12)
}

/// The activity dot for a status (FR8), or `None` when the session's activity
/// is unmonitored so the UI never shows a misleading dot. Shared by the tab
/// strip's chips and the sidebar's per-session dots so both stay in sync. The
/// colour is the only place the UI shows a status; the word form an agent
/// reads lives in [`crate::snapshot_dto`], not on the screen.
pub(super) fn status_dot<'a>(status: SessionStatus, backdrop: Backdrop) -> Option<Text<'a>> {
    let hue = status_hue(status)?;
    Some(
        text("●")
            .size(9)
            .style(move |theme: &iced::Theme| text::Style {
                color: Some(shade_status(hue, theme.extended_palette(), backdrop)),
            }),
    )
}

/// What a mark is drawn on, which decides the shade that reads there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Backdrop {
    /// The window's own surfaces, the [`mark_surfaces`].
    Chrome,
    /// The active tab chip's fill, [`active_chip_fill`].
    ActiveChip,
    /// A waiting tab chip's fill, [`waiting_chip_fill`].
    WaitingChip,
}

/// The hue of a waiting session's marks before any shading, tuned for a dark
/// surface. The document editor's error note shares it.
pub(super) const ATTENTION_HUE: Color = Color::from_rgb(0.95, 0.35, 0.35);

/// The hue of an idle session's dot before any shading; the document editor's
/// saved note shares it.
pub(super) const IDLE_HUE: Color = Color::from_rgb(0.3, 0.8, 0.4);

/// A status's hue before any shading, tuned for a dark surface, or `None`
/// when the status has no mark.
fn status_hue(status: SessionStatus) -> Option<Color> {
    match status {
        SessionStatus::Starting => Some(Color::from_rgb(0.55, 0.55, 0.6)),
        SessionStatus::Unmonitored => None,
        SessionStatus::Busy => Some(Color::from_rgb(0.95, 0.7, 0.2)),
        SessionStatus::Idle => Some(IDLE_HUE),
        SessionStatus::Attention => Some(ATTENTION_HUE),
        SessionStatus::Exited => Some(Color::from_rgb(0.5, 0.5, 0.5)),
    }
}

fn shade_status(hue: Color, palette: &Extended, backdrop: Backdrop) -> Color {
    match backdrop {
        Backdrop::Chrome => on_surface(hue, palette),
        Backdrop::ActiveChip => on_fill(hue, active_chip_fill(palette)),
        Backdrop::WaitingChip => on_fill(hue, waiting_chip_fill(palette)),
    }
}

#[cfg(test)]
fn status_color(status: SessionStatus, palette: &Extended, backdrop: Backdrop) -> Option<Color> {
    Some(shade_status(status_hue(status)?, palette, backdrop))
}

/// `hue` shaded to read on `fill`, pushed toward whichever of white and black
/// contrasts more with it. One of the two always clears [`MARK_FLOOR`] — the
/// better of them is at least 4.58:1 on any colour — so the push never runs
/// out. A fill has no theme-wide tuning to keep, unlike [`on_surface`].
fn on_fill(hue: Color, fill: Color) -> Color {
    on_fill_at(hue, fill, MARK_FLOOR)
}

/// [`on_fill`] with the contrast `floor` the shade must clear. The better of
/// white and black clears [`TEXT_FLOOR`] too.
fn on_fill_at(hue: Color, fill: Color, floor: f32) -> Color {
    let fill_luminance = luminance(fill);
    let away = if contrast_ratio(1.0, fill_luminance) >= contrast_ratio(0.0, fill_luminance) {
        Color::WHITE
    } else {
        Color::BLACK
    };
    shade_until_it_reads(hue, away, &[fill], floor)
}

/// The hues are tuned for a dark surface. The fixed light-theme darkening
/// suits the stock themes; a mid-tone surface (Nord, Tokyo Night Light) also
/// needs the push away from it, until the mark clears [`MARK_FLOOR`] on the
/// [`mark_surfaces`].
fn on_surface(hue: Color, palette: &Extended) -> Color {
    on_surface_at(hue, palette, MARK_FLOOR)
}

/// `hue` shaded to read as text on the window's surfaces: [`on_surface`]
/// held to [`TEXT_FLOOR`] rather than the mark floor.
pub(super) fn text_on_surface(hue: Color, palette: &Extended) -> Color {
    on_surface_at(hue, palette, TEXT_FLOOR)
}

/// [`on_surface`] with the contrast `floor` the shade must clear.
fn on_surface_at(hue: Color, palette: &Extended, floor: f32) -> Color {
    let (start, away) = if palette.is_dark {
        (hue, Color::WHITE)
    } else {
        (mix(hue, Color::BLACK, LIGHT_SURFACE_DARKEN), Color::BLACK)
    };
    shade_until_it_reads(start, away, &mark_surfaces(palette), floor)
}

/// The first shade on the way from `start` to `away` that clears `floor` on
/// every one of `surfaces`, else `away` itself.
fn shade_until_it_reads(start: Color, away: Color, surfaces: &[Color], floor: f32) -> Color {
    let surfaces: Vec<f32> = surfaces.iter().copied().map(luminance).collect();
    (0..=SHADE_STEPS)
        .map(|step| mix(start, away, f32::from(step) / f32::from(SHADE_STEPS)))
        .find(|shade| {
            let shade = luminance(*shade);
            surfaces.iter().all(|s| contrast_ratio(shade, *s) >= floor)
        })
        .unwrap_or(away)
}

/// The active tab chip's fill.
pub(super) fn active_chip_fill(palette: &Extended) -> Color {
    palette.primary.base.color
}

/// The fill of a tab chip whose tab waits for the user while another is
/// active: the strip's surface tinted toward the attention hue.
pub(super) fn waiting_chip_fill(palette: &Extended) -> Color {
    mix(palette.background.base.color, ATTENTION_HUE, WAITING_TINT)
}

/// How far a waiting chip's fill moves from the strip toward the attention
/// hue: enough to tell it from a plain chip at a glance, little enough to
/// stay a tint rather than an alarm.
const WAITING_TINT: f32 = 0.3;

/// A waiting tab chip's text, shaded to read on [`waiting_chip_fill`].
pub(super) fn waiting_chip_text(palette: &Extended) -> Color {
    on_fill_at(
        palette.background.base.text,
        waiting_chip_fill(palette),
        TEXT_FLOOR,
    )
}

/// The border of a split pane that waits for the user.
pub(super) fn waiting_ring(palette: &Extended) -> Color {
    shade_status(ATTENTION_HUE, palette, Backdrop::Chrome)
}

/// The window surfaces a mark must read on. The active tab chip's fill is a
/// backdrop of its own ([`Backdrop::ActiveChip`]): no one shade suits both.
fn mark_surfaces(palette: &Extended) -> [Color; 2] {
    [palette.background.base.color, palette.background.weak.color]
}

/// How far a hovered sidebar row moves from the sidebar: enough to show which
/// row the revealed actions belong to.
const ROW_HOVER_TINT: f32 = 0.12;

/// The fill of a hovered sidebar row: the sidebar's surface pushed away from
/// its text, so the hovered title reads at least as well as at rest. Several
/// themes sit at the text floor at rest; a tint toward the text would sink
/// them below it.
pub(super) fn row_hover_fill(palette: &Extended) -> Color {
    let surface = palette.background.weak.color;
    let away = if luminance(palette.background.base.text) > luminance(surface) {
        Color::BLACK
    } else {
        Color::WHITE
    };
    mix(surface, away, ROW_HOVER_TINT)
}

/// The WCAG contrast floor for body text.
pub(super) const TEXT_FLOOR: f32 = 4.5;

/// The contrast ratio of two colours, from 1 to 21.
#[cfg(test)]
pub(super) fn contrast(a: Color, b: Color) -> f32 {
    contrast_ratio(luminance(a), luminance(b))
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
    use crate::settings::ThemeChoice;

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

    /// Busy, the hue both cases in the report were measured with.
    const BUSY: SessionStatus = SessionStatus::Busy;

    const ALL: [SessionStatus; 5] = [
        SessionStatus::Starting,
        SessionStatus::Busy,
        SessionStatus::Idle,
        SessionStatus::Attention,
        SessionStatus::Exited,
    ];

    #[test]
    fn a_hovered_sidebar_row_reads_at_least_as_well_as_at_rest_on_every_theme() {
        for theme in every_theme() {
            let palette = theme.extended_palette();
            let (surface, fill) = (palette.background.weak.color, row_hover_fill(palette));
            let ink = palette.background.base.text;
            let (rest, hovered) = (contrast(ink, surface), contrast(ink, fill));
            assert!(
                hovered >= rest,
                "{theme}: title {rest:.2} at rest, {hovered:.2} hovered"
            );
            assert_ne!(fill, surface, "{theme}: no visible tint");
        }
    }

    #[test]
    fn every_status_dot_reads_on_every_themes_hovered_row() {
        for theme in every_theme() {
            let palette = theme.extended_palette();
            let fill = row_hover_fill(palette);
            for status in ALL {
                let dot = status_color(status, palette, Backdrop::Chrome).expect("monitored");
                let ratio = contrast(dot, fill);
                assert!(
                    ratio >= MARK_FLOOR,
                    "{status:?} on {theme}'s hovered row: {ratio:.2}"
                );
            }
        }
    }

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
                    let dot = status_color(status, palette, Backdrop::Chrome).expect("monitored");
                    let ratio = contrast(dot, surface);
                    assert!(ratio >= MARK_FLOOR, "{status:?} on {theme}: {ratio:.2}");
                }
            }
        }
    }

    #[test]
    fn every_status_dot_reads_on_every_themes_active_chip() {
        for theme in every_theme() {
            let palette = theme.extended_palette();
            let fill = active_chip_fill(palette);
            for status in ALL {
                let dot = status_color(status, palette, Backdrop::ActiveChip).expect("monitored");
                let ratio = contrast(dot, fill);
                assert!(
                    ratio >= MARK_FLOOR,
                    "{status:?} on {theme}'s chip: {ratio:.2}"
                );
            }
        }
    }

    #[test]
    fn every_status_dot_reads_on_every_themes_waiting_chip() {
        for theme in every_theme() {
            let palette = theme.extended_palette();
            let fill = waiting_chip_fill(palette);
            for status in ALL {
                let dot = status_color(status, palette, Backdrop::WaitingChip).expect("monitored");
                let ratio = contrast(dot, fill);
                assert!(
                    ratio >= MARK_FLOOR,
                    "{status:?} on {theme}'s waiting chip: {ratio:.2}"
                );
            }
        }
    }

    #[test]
    fn a_waiting_chips_title_reads_as_text_on_every_theme() {
        for theme in every_theme() {
            let palette = theme.extended_palette();
            let ratio = contrast(waiting_chip_text(palette), waiting_chip_fill(palette));
            assert!(ratio >= TEXT_FLOOR, "{theme}: {ratio:.2}");
        }
    }

    /// How much redder than green `c` is: the attention hue's lean.
    fn red_lean(c: Color) -> f32 {
        c.r - c.g
    }

    #[test]
    fn a_waiting_chip_leans_red_against_its_strip_on_every_theme() {
        for theme in every_theme() {
            let palette = theme.extended_palette();
            let fill = waiting_chip_fill(palette);
            let strip = palette.background.base.color;
            assert!(
                red_lean(fill) >= red_lean(strip) + 0.1,
                "{theme}: fill {fill:?} on strip {strip:?}"
            );
        }
    }

    #[test]
    fn the_waiting_ring_is_the_attention_mark_on_every_theme() {
        for theme in every_theme() {
            let palette = theme.extended_palette();
            assert_eq!(
                Some(waiting_ring(palette)),
                status_color(SessionStatus::Attention, palette, Backdrop::Chrome),
                "{theme}"
            );
        }
    }

    #[test]
    fn the_reported_chips_now_read() {
        // The two rows the report measured: about 2.5:1 and 1.3:1 before.
        for theme in [iced::Theme::Dark, iced::Theme::Dracula] {
            let palette = theme.extended_palette();
            let fill = active_chip_fill(palette);
            let chrome = status_color(BUSY, palette, Backdrop::Chrome).expect("monitored");
            assert!(
                contrast(chrome, fill) < MARK_FLOOR,
                "the chrome shade must fail on {theme}'s chip, or this proves nothing"
            );
            let dot = status_color(BUSY, palette, Backdrop::ActiveChip).expect("monitored");
            assert!(contrast(dot, fill) >= MARK_FLOOR, "{theme}");
        }
    }

    #[test]
    fn a_chip_dot_is_pushed_toward_the_extreme_that_contrasts_with_the_fill() {
        // Dark's chip is a deep blue, Dracula's a pale violet.
        let dark = iced::Theme::Dark.extended_palette();
        let dracula = iced::Theme::Dracula.extended_palette();
        let hue = luminance(Color::from_rgb(0.95, 0.7, 0.2));
        assert!(
            luminance(status_color(BUSY, dark, Backdrop::ActiveChip).expect("monitored")) > hue
        );
        assert!(
            luminance(status_color(BUSY, dracula, Backdrop::ActiveChip).expect("monitored")) < hue
        );
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

    /// The six stock themes paint what they did before the push existed.
    #[test]
    fn the_stock_themes_keep_their_tuned_colours() {
        // (hue, painted on a light stock theme); a dark one paints the hue.
        let cases = [
            ([0.95, 0.7, 0.2], [0.57, 0.42, 0.12]),
            ([1.0, 0.42, 0.40], [0.6, 0.252, 0.24]),
        ];
        for theme in ThemeChoice::ALL[..6].iter().map(|t| t.to_iced()) {
            let palette = theme.extended_palette();
            for ([r, g, b], light) in cases {
                let [er, eg, eb] = if palette.is_dark { [r, g, b] } else { light };
                let got = on_surface(Color::from_rgb(r, g, b), palette);
                for (got, want) in [(got.r, er), (got.g, eg), (got.b, eb)] {
                    assert!((got - want).abs() < 1e-4, "{theme}: {got} vs {want}");
                }
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
            let on_dark = status_color(status, dark, Backdrop::Chrome).expect("monitored");
            let on_light = status_color(status, light, Backdrop::Chrome).expect("monitored");
            assert!(
                luminance(on_light) < luminance(on_dark),
                "{status:?} darkens"
            );
        }
    }

    #[test]
    fn unmonitored_status_omits_dot_and_every_kind_has_a_distinct_glyph() {
        let dark = iced::Theme::Dark.extended_palette();
        assert!(status_color(SessionStatus::Unmonitored, dark, Backdrop::Chrome).is_none());
        assert!(status_dot(SessionStatus::Unmonitored, Backdrop::Chrome).is_none());
        for status in ALL {
            assert!(status_dot(status, Backdrop::Chrome).is_some());
        }
        let glyphs: std::collections::HashSet<&str> = [
            SessionKind::Shell,
            SessionKind::Agent(Provider::Claude),
            SessionKind::Agent(Provider::Codex),
            SessionKind::Agent(Provider::Antigravity),
        ]
        .into_iter()
        .map(kind_glyph)
        .collect();
        assert_eq!(glyphs.len(), 4);
    }
}
