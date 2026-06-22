//! Window-bounds persistence (FR12) — a tiny file adapter owned by the
//! shell until the real `store` adapter lands in M1. Lives in `app`
//! because it is I/O; `core` never sees it.

use std::path::PathBuf;

use display_info::DisplayInfo;
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WindowConfig {
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 1100.0,
            height: 720.0,
            x: None,
            y: None,
        }
    }
}

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
#[must_use]
pub fn current_screens() -> Vec<ScreenRect> {
    match DisplayInfo::all() {
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

impl WindowConfig {
    /// Load persisted bounds; any problem (no file, bad JSON) falls back to
    /// defaults — a corrupt config must never prevent startup.
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str::<Self>(&raw)
                .map(Self::sanitised)
                .unwrap_or_else(|e| {
                    warn!(error = %e, path = %path.display(), "invalid window config; using defaults");
                    Self::default()
                }),
            Err(_) => Self::default(),
        }
    }

    /// Drop bounds that would place the window off-screen or give it a
    /// degenerate size. A minimised window reports a `(0,0)` size and a huge
    /// negative position; persisting those leaves the window invisible on the
    /// next launch. Applied on both load and save so neither a fresh garbage
    /// value nor an already-corrupt file can hide the window.
    fn sanitised(mut self) -> Self {
        let default = Self::default();
        if !(self.width >= MIN_DIM && self.height >= MIN_DIM) {
            self.width = default.width;
            self.height = default.height;
        }
        let off_screen = |v: f32| !v.is_finite() || v.abs() > POS_LIMIT;
        if self.x.is_some_and(off_screen) || self.y.is_some_and(off_screen) {
            self.x = None;
            self.y = None;
        }
        self
    }

    /// Drop the saved position when it would open the window off every
    /// connected monitor — e.g. a position remembered on a second monitor that
    /// has since been unplugged. Cleared bounds fall back to
    /// `Position::Centered`, which winit places on a real monitor.
    ///
    /// `screens` empty means the monitor query failed (or is unavailable): we
    /// cannot validate, so the saved position is trusted as-is rather than
    /// risking a needless recenter. A genuine size that simply doesn't fit is
    /// left alone; only the *position* is discarded.
    #[must_use]
    pub fn with_onscreen_position(mut self, screens: &[ScreenRect]) -> Self {
        if screens.is_empty() {
            return self;
        }
        if let (Some(x), Some(y)) = (self.x, self.y) {
            let window = ScreenRect {
                x,
                y,
                width: self.width,
                height: self.height,
            };
            if !position_is_reachable(window, screens) {
                self.x = None;
                self.y = None;
            }
        }
        self
    }

    /// Persist bounds. Failures are logged, never fatal — losing window
    /// geometry is not worth blocking shutdown.
    pub fn save(self) {
        let self_ = self.sanitised();
        let Some(path) = config_path() else {
            return;
        };
        if let Some(dir) = path.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            warn!(error = %e, "could not create config dir");
            return;
        }
        match serde_json::to_string_pretty(&self_) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    warn!(error = %e, path = %path.display(), "could not save window config");
                }
            }
            Err(e) => warn!(error = %e, "could not serialise window config"),
        }
    }
}

/// Whether enough of `window` overlaps any monitor to be grabbed and dragged
/// back into view — at least [`MIN_VISIBLE_W`] × [`MIN_VISIBLE_H`] logical px on
/// a single screen. Pure geometry, so the off-screen rule is unit-testable.
fn position_is_reachable(window: ScreenRect, screens: &[ScreenRect]) -> bool {
    screens.iter().any(|screen| {
        let overlap_w =
            (window.x + window.width).min(screen.x + screen.width) - window.x.max(screen.x);
        let overlap_h =
            (window.y + window.height).min(screen.y + screen.height) - window.y.max(screen.y);
        overlap_w >= MIN_VISIBLE_W && overlap_h >= MIN_VISIBLE_H
    })
}

/// `~/.termherd/window.json` — the app data dir from the PRD (§7).
fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".termherd").join("window.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimised_geometry_is_dropped() {
        // The minimised-window sentinel: zero size, huge negative position.
        let garbage = WindowConfig {
            width: 0.0,
            height: 0.0,
            x: Some(-25600.0),
            y: Some(-25600.0),
        }
        .sanitised();
        let default = WindowConfig::default();
        assert_eq!(garbage.width, default.width);
        assert_eq!(garbage.height, default.height);
        assert_eq!(garbage.x, None);
        assert_eq!(garbage.y, None);
    }

    #[test]
    fn valid_geometry_is_preserved() {
        let good = WindowConfig {
            width: 1280.0,
            height: 800.0,
            x: Some(40.0),
            y: Some(60.0),
        };
        let kept = good.sanitised();
        assert_eq!(kept.width, 1280.0);
        assert_eq!(kept.height, 800.0);
        assert_eq!(kept.x, Some(40.0));
        assert_eq!(kept.y, Some(60.0));
    }

    fn screen(x: f32, y: f32, width: f32, height: f32) -> ScreenRect {
        ScreenRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn position_on_a_disconnected_monitor_is_cleared() {
        // The reported bug: a window remembered at x=2552 on a second monitor
        // that is no longer connected. Only the primary (0..2560) remains, and
        // just 8 px of the window fall on it — not reachable, so the position is
        // dropped and the window re-centers.
        let cfg = WindowConfig {
            width: 1920.0,
            height: 1009.0,
            x: Some(2552.0),
            y: Some(-8.0),
        }
        .with_onscreen_position(&[screen(0.0, 0.0, 2560.0, 1440.0)]);
        assert_eq!(cfg.x, None);
        assert_eq!(cfg.y, None);
        // The size the user chose is untouched — only the position is dropped.
        assert_eq!(cfg.width, 1920.0);
        assert_eq!(cfg.height, 1009.0);
    }

    #[test]
    fn position_on_a_connected_secondary_monitor_is_kept() {
        // A genuine multi-monitor layout: the window sits on a second monitor to
        // the right (origin 2560). It must NOT be recentered — the regression a
        // naive (0,0)-origin check would cause.
        let screens = [
            screen(0.0, 0.0, 2560.0, 1440.0),
            screen(2560.0, 0.0, 1920.0, 1080.0),
        ];
        let cfg = WindowConfig {
            width: 1280.0,
            height: 800.0,
            x: Some(2700.0),
            y: Some(100.0),
        }
        .with_onscreen_position(&screens);
        assert_eq!(cfg.x, Some(2700.0));
        assert_eq!(cfg.y, Some(100.0));
    }

    #[test]
    fn an_empty_monitor_list_leaves_the_position_untouched() {
        // Enumeration failed: we cannot validate, so trust the saved position
        // rather than needlessly recentering a possibly-fine window.
        let cfg = WindowConfig {
            width: 1000.0,
            height: 700.0,
            x: Some(4000.0),
            y: Some(4000.0),
        }
        .with_onscreen_position(&[]);
        assert_eq!(cfg.x, Some(4000.0));
        assert_eq!(cfg.y, Some(4000.0));
    }

    #[test]
    fn a_sliver_on_screen_still_counts_as_offscreen() {
        // A window peeking only a few px onto the monitor can't be grabbed by
        // its title bar, so it is treated as off-screen.
        let cfg = WindowConfig {
            width: 1000.0,
            height: 700.0,
            x: Some(-960.0), // only 40 px (< MIN_VISIBLE_W) overlap at the left
            y: Some(100.0),
        }
        .with_onscreen_position(&[screen(0.0, 0.0, 1920.0, 1080.0)]);
        assert_eq!(cfg.x, None);
        assert_eq!(cfg.y, None);
    }

    #[test]
    fn off_screen_position_resets_but_keeps_good_size() {
        let cfg = WindowConfig {
            width: 1000.0,
            height: 700.0,
            x: Some(f32::NAN),
            y: Some(500.0),
        }
        .sanitised();
        assert_eq!(cfg.width, 1000.0);
        assert_eq!(cfg.x, None);
        assert_eq!(cfg.y, None);
    }
}
