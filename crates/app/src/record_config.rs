//! The GIF screencast budget — the runtime configuration shared by the
//! recorder thread ([`crate::record`]), the shell, and the settings layer.
//! Hoisted out of the recorder so config consumers don't depend on the
//! encoding I/O, and [`crate::settings`] converts into it from one source.

use std::time::Duration;

/// Recording budget — frames per second, hard duration cap, and the scale the
/// captured frames are downsampled to. The default (8 fps / 30 s / 0.5×) keeps
/// GIFs manageable while smooth enough for a bug repro; configurable via the
/// `record` block in `settings.json`.
#[derive(Debug, Clone, Copy)]
pub struct RecordConfig {
    pub fps: u32,
    pub max_seconds: u32,
    pub scale: f32,
}

impl Default for RecordConfig {
    fn default() -> Self {
        Self {
            fps: 8,
            max_seconds: 30,
            scale: 0.5,
        }
    }
}

impl RecordConfig {
    /// The frame cap the `core` state machine counts against (`fps × seconds`).
    #[must_use]
    pub fn max_frames(&self) -> u32 {
        self.fps.saturating_mul(self.max_seconds)
    }

    /// The nominal gap between frames at the target fps — the first frame's
    /// on-screen duration (it has no predecessor to measure against) and the
    /// throttle interval.
    #[must_use]
    pub fn frame_interval(&self) -> Duration {
        Duration::from_secs_f32(1.0 / self.fps.max(1) as f32)
    }
}
