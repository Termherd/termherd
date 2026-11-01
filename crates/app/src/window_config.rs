//! Window-bounds persistence (FR12) — a tiny file adapter owned by the
//! shell until the real `store` adapter lands in M1. Lives in `app`
//! because it is I/O; `core` never sees it. The file plumbing is
//! [`crate::json_store`], the geometry rules [`crate::window_geometry`].

use serde::{Deserialize, Serialize};

use crate::window_geometry::{ScreenRect, position_is_reachable, sanitised};

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

impl WindowConfig {
    /// Load persisted bounds; any problem (no file, bad JSON) falls back to
    /// defaults — a corrupt config must never prevent startup. Loaded bounds
    /// are sanitised so an already-corrupt file can't hide the window.
    #[must_use]
    pub fn load() -> Self {
        sanitised(crate::json_store::load_json(FILE))
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

    /// Persist bounds, sanitised so fresh garbage (a minimised window's
    /// sentinel geometry) is never written. Failures are logged, never fatal —
    /// losing window geometry is not worth blocking shutdown.
    pub fn save(self) {
        crate::json_store::save_json(FILE, &sanitised(self));
    }
}

/// `~/.termherd/window.json` — the app data dir from the PRD (§7).
const FILE: &str = "window.json";

#[cfg(test)]
mod tests {
    use super::*;

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
}
