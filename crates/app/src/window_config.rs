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

impl WindowConfig {
    /// Load persisted bounds; any problem (no file, bad JSON) falls back to
    /// defaults — a corrupt config must never prevent startup.
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
                warn!(error = %e, path = %path.display(), "invalid window config; using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Persist bounds. Failures are logged, never fatal — losing window
    /// geometry is not worth blocking shutdown.
    pub fn save(self) {
        let Some(path) = config_path() else {
            return;
        };
        if let Some(dir) = path.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            warn!(error = %e, "could not create config dir");
            return;
        }
        match serde_json::to_string_pretty(&self) {
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
