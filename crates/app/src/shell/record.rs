//! GIF screencast state (F-capture rung 2): the in-progress recording's
//! runtime state and the executor that drives the off-thread encoder. Split
//! from the shell's state machine so the recording concern — its budget, its
//! frame throttle, and the start→feed→finish lifecycle — is one self-contained
//! object the shell holds a single field of.

use std::time::{Instant, SystemTime};

use iced::{Task, window};
use termherd_core::Effect;

use super::{Message, Shell};
use crate::record::{FrameStats, FrameThrottle, RecordConfig, Recorder};

/// The runtime state of the GIF screencast: the immutable frame budget plus
/// everything an in-progress recording needs. Idle when `recorder` is `None`.
pub(super) struct RecordState {
    /// The GIF screencast budget: fps / duration cap / frame scale.
    /// Default for now; `settings.json` configurability is a follow-up.
    config: RecordConfig,
    /// The recorder thread for an in-progress screencast, or `None`. The
    /// encoder lives off the UI thread; the shell only feeds it frames.
    recorder: Option<Recorder>,
    /// Frame screenshots requested but not yet handed to the recorder.
    /// A stop waits for this to drain so the final frames aren't lost.
    pub(super) inflight: u32,
    /// A finish is pending until the last in-flight frame is handed off.
    pub(super) finish_pending: bool,
    /// When the in-progress recording started — the origin for the
    /// throttle's logical timeline. `None` when not recording.
    started: Option<Instant>,
    /// Throttles the window's present-rate frame source down to the configured
    /// fps. `None` when not recording.
    throttle: Option<FrameThrottle>,
    /// When the previous frame was handed to the encoder, so each frame's
    /// on-screen duration is the real wall-clock gap since the last one.
    last_frame: Option<Instant>,
    /// Per-frame gap statistics for the in-progress recording — logged at
    /// stop to evidence real-time capture (vs the idle-window time-lapse).
    stats: FrameStats,
}

impl RecordState {
    pub(super) fn new(config: RecordConfig) -> Self {
        Self {
            config,
            recorder: None,
            inflight: 0,
            finish_pending: false,
            started: None,
            throttle: None,
            last_frame: None,
            stats: FrameStats::default(),
        }
    }

    /// The frame cap `core` needs to decide the auto-stop.
    pub(super) fn max_frames(&self) -> u32 {
        self.config.max_frames()
    }

    /// Whether a ⌘⇧R press must be ignored because the previous recording is
    /// still draining its in-flight frames (problem 2). `core` has already
    /// returned to idle by the time a finish is pending, so without this guard a
    /// back-to-back start replaces `self.recorder` mid-finish — orphaning the
    /// first GIF (it finalises via `Drop`, but logs no `screencast written` and
    /// may be truncated).
    pub(super) fn toggle_blocked(&self) -> bool {
        self.finish_pending
    }

    /// Perform the record effects `core` returned: open/feed/finish the
    /// encoder thread. `CaptureFrame` is the only one with an async follow-up —
    /// it screenshots the window, the result arriving as [`Message::RecordFrame`].
    pub(super) fn run_effects(&mut self, effects: Vec<Effect>) -> Task<Message> {
        let mut task = Task::none();
        for effect in effects {
            match effect {
                Effect::StartRecording => self.start(),
                Effect::CaptureFrame => {
                    self.inflight += 1;
                    task = window::latest()
                        .and_then(window::screenshot)
                        .map(Message::RecordFrame);
                }
                // `core` names the stop reason; logged the moment it happens (not
                // after the encoder drains) so start↔stop is unambiguous in the
                // trace.
                Effect::FinishRecording { capped } => {
                    let reason = if capped { "cap reached" } else { "manual" };
                    tracing::info!(reason, "screencast recording stopped");
                    self.request_finish();
                }
                Effect::CancelRecording => {
                    tracing::info!("screencast recording cancelled (no frames captured)");
                    self.cancel();
                }
                _ => {}
            }
        }
        task
    }

    /// Open the recorder thread for a new screencast, writing to
    /// `capture-<ts>.gif` in the capture dir. A missing home dir or an
    /// uncreatable dir aborts the start — logged, never fatal.
    fn start(&mut self) {
        self.inflight = 0;
        self.finish_pending = false;
        // Fresh timing state for the throttle, the per-frame gap, and the stats.
        self.started = Some(Instant::now());
        self.throttle = Some(FrameThrottle::new(self.config.frame_interval()));
        self.last_frame = None;
        self.stats = FrameStats::default();
        let Some(dir) = crate::capture::captures_dir() else {
            tracing::warn!("no home directory; recording skipped");
            return;
        };
        if let Err(error) = std::fs::create_dir_all(&dir) {
            tracing::warn!(%error, "could not create captures dir; recording skipped");
            return;
        }
        let stamp = crate::capture::stamp(SystemTime::now());
        let path = dir.join(format!("capture-{stamp}.gif"));
        self.recorder = Some(Recorder::start(path, self.config));
        tracing::info!(
            fps = self.config.fps,
            cap_frames = self.config.max_frames(),
            "screencast recording started"
        );
    }

    /// Whether this present tick clears the fps throttle: on a kept tick the
    /// shell asks `core` for the next frame / auto-stop decision. Skipped ticks
    /// are dropped — they only served to keep the window presenting so the
    /// screenshot pipeline stays real-time.
    pub(super) fn should_capture_tick(&mut self, now: Instant) -> bool {
        let (Some(started), Some(throttle)) = (self.started, self.throttle.as_mut()) else {
            return false;
        };
        throttle.should_capture(now.saturating_duration_since(started))
    }

    /// Finish the screencast once every in-flight frame screenshot has been
    /// handed to the encoder, so a stop never drops the final frames. If
    /// none are in flight (a manual stop), finish straight away.
    fn request_finish(&mut self) {
        if self.inflight == 0 {
            self.finish();
        } else {
            self.finish_pending = true;
        }
    }

    /// Flush and close the encoder thread, logging the frame-gap spread
    /// — the evidence that capture ran in real time (gaps ≈ the interval)
    /// rather than time-lapsed (gaps ballooning past it).
    fn finish(&mut self) {
        if let Some(recorder) = self.recorder.take() {
            recorder.finish();
        }
        if let Some(summary) = self.stats.summary() {
            tracing::info!(
                frames = summary.gaps,
                min_ms = summary.min.as_millis(),
                max_ms = summary.max.as_millis(),
                mean_ms = summary.mean.as_millis(),
                "screencast frame gaps"
            );
        }
        self.reset();
    }

    /// Abandon the screencast, deleting any partial file — the zero-frame
    /// stop.
    fn cancel(&mut self) {
        if let Some(recorder) = self.recorder.take() {
            recorder.cancel();
        }
        self.reset();
    }

    /// Clear the per-recording runtime state once the encoder is done,
    /// returning to idle.
    fn reset(&mut self) {
        self.inflight = 0;
        self.finish_pending = false;
        self.started = None;
        self.throttle = None;
        self.last_frame = None;
        self.stats = FrameStats::default();
    }

    /// Hand one recorded window screenshot to the encoder thread, then
    /// finish if this was the last frame a stop was waiting on. The gap since the
    /// previous handed frame becomes this frame's on-screen duration, and
    /// is folded into the stats — the gaps are the present-gating measurement.
    pub(super) fn on_frame(&mut self, screenshot: window::Screenshot) -> Task<Message> {
        let now = Instant::now();
        let previous = self.last_frame;
        // The first frame has no predecessor, so it shows for the nominal
        // interval; later frames show for the real gap since the last one.
        let gap = previous.map_or_else(
            || self.config.frame_interval(),
            |last| now.saturating_duration_since(last),
        );
        if let Some(recorder) = self.recorder.as_ref() {
            let (width, height) = (screenshot.size.width, screenshot.size.height);
            recorder.frame(screenshot.rgba.to_vec(), width, height, gap);
            // Only *measured* gaps count toward the benchmark — the first frame's
            // nominal interval isn't a real arrival gap and would skew min/mean.
            if previous.is_some() {
                self.stats.record_gap(gap);
            }
            self.last_frame = Some(now);
        }
        self.inflight = self.inflight.saturating_sub(1);
        if self.finish_pending && self.inflight == 0 {
            self.finish();
        }
        Task::none()
    }
}

impl Shell {
    /// Start or stop the GIF screencast: hand `core` the frame cap from the
    /// record budget and perform whatever effects it returns. Ignored while a
    /// previous recording is still draining, so a back-to-back ⌘⇧R can't replace
    /// the recorder mid-finish.
    pub(super) fn toggle_record(&mut self) -> Task<Message> {
        if self.record.toggle_blocked() {
            tracing::info!("record toggle ignored: previous screencast still finishing");
            return Task::none();
        }
        let max_frames = self.record.max_frames();
        let effects = self
            .core
            .apply(termherd_core::Event::ToggleRecord { max_frames });
        self.record.run_effects(effects)
    }

    /// A window present arrived while recording: throttle the present rate down
    /// to the configured fps and, on a kept tick, ask `core` for the next frame
    /// / auto-stop decision.
    pub(super) fn on_record_frame_tick(&mut self, now: Instant) -> Task<Message> {
        if !self.record.should_capture_tick(now) {
            return Task::none();
        }
        let effects = self.core.apply(termherd_core::Event::RecordTick);
        self.record.run_effects(effects)
    }
}
