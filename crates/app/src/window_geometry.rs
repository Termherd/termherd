//! Monitor geometry and window-bounds sanity (FR12) — the pure half of the
//! window persistence split: which monitors exist, whether saved bounds are
//! degenerate or unreachable. [`crate::window_config`] owns the file; this
//! owns the geometry, kept separate so the rules are unit-testable without a
//! display server.

use tracing::warn;

use crate::window_config::WindowConfig;

/// Smallest window we will ever restore to; anything below this (a `(0,0)`
/// resize emitted while minimising) is treated as garbage.
const MIN_DIM: f32 = 200.0;
/// Positions outside this logical range are off every plausible monitor — a
/// minimised window reports the Windows sentinel (`-32000` physical, e.g.
/// `-25600` logical at 125% DPI), which must never be persisted as a position.
const POS_LIMIT: f32 = 20_000.0;
/// How much of the window must fall on a monitor for it to count as reachable:
/// enough of the title bar to grab and drag it back. A thinner sliver — e.g.
/// the few visible px left after unplugging a second monitor — is treated as
/// off-screen.
const MIN_VISIBLE_W: f32 = 80.0;
const MIN_VISIBLE_H: f32 = 24.0;

/// A monitor's bounds in **logical** pixels — the same space iced reports the
/// window position in (`to_logical(scale_factor)`). Kept tiny and pure so the
/// reachability decision is unit-testable without touching the display server.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// The currently connected monitors, in logical pixels. `display-info` reports
/// physical bounds plus the per-monitor scale, so each rect is divided back to
/// logical to match iced's window coordinates. Returns empty on any enumeration
/// failure — the caller treats "no monitors known" as "cannot validate" and
/// keeps the saved position untouched, never hiding a window because the query
/// failed.
#[cfg(not(target_os = "linux"))]
#[must_use]
pub fn current_screens() -> Vec<ScreenRect> {
    match display_info::DisplayInfo::all() {
        Ok(displays) => displays
            .iter()
            .map(|d| {
                let scale = if d.scale_factor > 0.0 {
                    d.scale_factor
                } else {
                    1.0
                };
                ScreenRect {
                    x: d.x as f32 / scale,
                    y: d.y as f32 / scale,
                    width: d.width as f32 / scale,
                    height: d.height as f32 / scale,
                }
            })
            .collect(),
        Err(e) => {
            warn!(error = %e, "could not enumerate monitors; keeping saved window position");
            Vec::new()
        }
    }
}

/// Linux has no monitor enumeration here: `display-info`'s X11 backend
/// hard-links libxcb, which the project avoids (dlopen-only X11/Wayland, lighter
/// build/.deb). Returning empty makes the caller trust the saved position — the
/// pre-existing behaviour — so the off-screen guard is a Windows/macOS feature
/// for now, where the reported bug occurs.
#[cfg(target_os = "linux")]
#[must_use]
pub fn current_screens() -> Vec<ScreenRect> {
    Vec::new()
}

/// Drop bounds that would place the window off-screen or give it a
/// degenerate size. A minimised window reports a `(0,0)` size and a huge
/// negative position; persisting those leaves the window invisible on the
/// next launch. Applied on both load and save so neither a fresh garbage
/// value nor an already-corrupt file can hide the window.
#[must_use]
pub fn sanitised(mut config: WindowConfig) -> WindowConfig {
    let default = WindowConfig::default();
    if !(config.width >= MIN_DIM && config.height >= MIN_DIM) {
        config.width = default.width;
        config.height = default.height;
    }
    let off_screen = |v: f32| !v.is_finite() || v.abs() > POS_LIMIT;
    if config.x.is_some_and(off_screen) || config.y.is_some_and(off_screen) {
        config.x = None;
        config.y = None;
    }
    config
}

/// Whether enough of `window` overlaps any monitor to be grabbed and dragged
/// back into view — at least [`MIN_VISIBLE_W`] × [`MIN_VISIBLE_H`] logical px on
/// a single screen. Pure geometry, so the off-screen rule is unit-testable.
pub fn position_is_reachable(window: ScreenRect, screens: &[ScreenRect]) -> bool {
    screens.iter().any(|screen| {
        let overlap_w =
            (window.x + window.width).min(screen.x + screen.width) - window.x.max(screen.x);
        let overlap_h =
            (window.y + window.height).min(screen.y + screen.height) - window.y.max(screen.y);
        overlap_w >= MIN_VISIBLE_W && overlap_h >= MIN_VISIBLE_H
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimised_geometry_is_dropped() {
        // The minimised-window sentinel: zero size, huge negative position.
        let garbage = sanitised(WindowConfig {
            width: 0.0,
            height: 0.0,
            x: Some(-25600.0),
            y: Some(-25600.0),
        });
        let default = WindowConfig::default();
        assert_eq!(garbage.width, default.width);
        assert_eq!(garbage.height, default.height);
        assert_eq!(garbage.x, None);
        assert_eq!(garbage.y, None);
    }

    #[test]
    fn valid_geometry_is_preserved() {
        let kept = sanitised(WindowConfig {
            width: 1280.0,
            height: 800.0,
            x: Some(40.0),
            y: Some(60.0),
        });
        assert_eq!(kept.width, 1280.0);
        assert_eq!(kept.height, 800.0);
        assert_eq!(kept.x, Some(40.0));
        assert_eq!(kept.y, Some(60.0));
    }

    #[test]
    fn off_screen_position_resets_but_keeps_good_size() {
        let cfg = sanitised(WindowConfig {
            width: 1000.0,
            height: 700.0,
            x: Some(f32::NAN),
            y: Some(500.0),
        });
        assert_eq!(cfg.width, 1000.0);
        assert_eq!(cfg.x, None);
        assert_eq!(cfg.y, None);
    }
}
