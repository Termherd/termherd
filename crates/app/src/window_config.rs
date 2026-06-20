//! Window-bounds persistence (FR12) — a tiny file adapter owned by the
//! shell until the real `store` adapter lands in M1. Lives in `app`
//! because it is I/O; `core` never sees it.

use std::path::PathBuf;

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
