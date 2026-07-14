//! Record adapter — the GIF encoder side of the screencast (F-capture
//! rung 2). `core` owns the idle→recording state machine; this module owns the
//! I/O it keeps out: the frame buffer, the resample, the `gif` encoder, and the
//! file.
//!
//! Encoding (a NeuQuant palette reduction per frame) is CPU-heavy, so it runs on
//! a **dedicated recorder thread** fed over a channel — the UI thread only hands
//! off raw frames. Keeping the GUI free also keeps the *recording* smooth, since
//! a stalled UI would be captured in its own frames.
//!
//! Output: `~/.termherd/captures/capture-<ts>.gif`, the capture dir.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use tracing::{info, warn};

use crate::record_config::RecordConfig;

/// A frame or lifecycle signal sent to the recorder thread.
enum RecordMsg {
    /// One captured frame: raw RGBA, its physical pixel size, and the wall-clock
    /// gap since the previous frame — the time this frame should stay on screen,
    /// so playback is real-time.
    Frame {
        rgba: Vec<u8>,
        width: u32,
        height: u32,
        gap: Duration,
    },
    /// Flush and close the GIF — all frames are in.
    Finish,
    /// Abandon the recording, deleting any partial file.
    Cancel,
}

/// Handle to a running recorder thread: the channel the UI thread feeds frames
/// down. Dropping the sender (or sending [`RecordMsg::Finish`]/`Cancel`) ends
/// the thread.
pub struct Recorder {
    tx: Sender<RecordMsg>,
}

impl Recorder {
    /// Start a recorder writing to `path` with `config`. The encoder opens
    /// lazily on the first frame (its dimensions come from that frame).
    #[must_use]
    pub fn start(path: PathBuf, config: RecordConfig) -> Self {
        let (tx, rx) = mpsc::channel::<RecordMsg>();
        // Detached: the thread finalises the file on Finish/Cancel and exits.
        std::thread::spawn(move || run(&rx, &path, config));
        Self { tx }
    }

    /// Hand one captured frame to the encoder thread: raw RGBA, its physical
    /// size, and `gap` — the wall-clock time since the previous frame, which
    /// becomes this frame's on-screen duration.
    pub fn frame(&self, rgba: Vec<u8>, width: u32, height: u32, gap: Duration) {
        let _ = self.tx.send(RecordMsg::Frame {
            rgba,
            width,
            height,
            gap,
        });
    }

    /// Flush and close the GIF.
    pub fn finish(self) {
        let _ = self.tx.send(RecordMsg::Finish);
    }

    /// Abandon the recording, deleting any partial file.
    pub fn cancel(self) {
        let _ = self.tx.send(RecordMsg::Cancel);
    }
}

/// The recorder thread loop: resample → quantise → write each frame, then
/// finalise. All failures are logged, never fatal — a broken capture must not
/// take the app down.
fn run(rx: &mpsc::Receiver<RecordMsg>, path: &PathBuf, config: RecordConfig) {
    let mut session: Option<Session> = None;
    while let Ok(msg) = rx.recv() {
        match msg {
            RecordMsg::Frame {
                rgba,
                width,
                height,
                gap,
            } => {
                if session.is_none() {
                    let (tw, th) = target_dims(width, height, config.scale);
                    match Session::open(path, tw, th) {
                        Ok(opened) => session = Some(opened),
                        Err(error) => {
                            warn!(%error, "could not open gif encoder; recording dropped");
                            return;
                        }
                    }
                }
                if let Some(session) = session.as_mut()
                    && let Err(error) = session.write(&rgba, width, height, gap)
                {
                    warn!(%error, "could not write gif frame");
                }
            }
            RecordMsg::Finish => {
                if let Some(session) = session.take() {
                    match session.finish() {
                        Ok(()) => info!(path = %path.display(), "screencast written"),
                        Err(error) => warn!(%error, "could not finalise screencast"),
                    }
                }
                return;
            }
            RecordMsg::Cancel => {
                let _ = std::fs::remove_file(path);
                return;
            }
        }
    }
}

/// An open GIF being written: the encoder plus the locked output dimensions
/// (set from the first frame, so a mid-record window resize is resampled to
/// fit, not corrupted). Each frame carries its own delay, so none is
/// stored here.
struct Session {
    encoder: gif::Encoder<BufWriter<File>>,
    target: (u32, u32),
}

impl Session {
    fn open(path: &PathBuf, tw: u32, th: u32) -> std::io::Result<Self> {
        let writer = BufWriter::new(File::create(path)?);
        let mut encoder =
            gif::Encoder::new(writer, tw as u16, th as u16, &[]).map_err(std::io::Error::other)?;
        encoder
            .set_repeat(gif::Repeat::Infinite)
            .map_err(std::io::Error::other)?;
        Ok(Self {
            encoder,
            target: (tw, th),
        })
    }

    fn write(&mut self, rgba: &[u8], sw: u32, sh: u32, gap: Duration) -> std::io::Result<()> {
        let (tw, th) = self.target;
        // Resample to the locked size only when the frame differs from it.
        let mut pixels = if (sw, sh) == (tw, th) {
            rgba.to_vec()
        } else {
            resample_nearest(rgba, sw, sh, tw, th)
        };
        // `from_rgba_speed` panics on a length mismatch; resample guarantees
        // exactly `tw*th*4`, so this holds. speed 10 balances quality/CPU.
        let mut frame = gif::Frame::from_rgba_speed(tw as u16, th as u16, &mut pixels, 10);
        // Real-time playback: this frame stays on screen for the wall-clock gap
        // since the previous one.
        frame.delay = gap_to_delay_cs(gap);
        self.encoder
            .write_frame(&frame)
            .map_err(std::io::Error::other)
    }

    fn finish(self) -> std::io::Result<()> {
        // `into_inner` writes the GIF trailer; then flush the BufWriter.
        let mut writer = self.encoder.into_inner().map_err(std::io::Error::other)?;
        writer.flush()
    }
}

/// The locked output dimensions for a source frame at `scale`, each at least 1
/// pixel and clamped to the GIF `u16` ceiling.
fn target_dims(sw: u32, sh: u32, scale: f32) -> (u32, u32) {
    let scaled = |n: u32| ((n as f32 * scale).round() as u32).clamp(1, u32::from(u16::MAX));
    (scaled(sw), scaled(sh))
}

/// Nearest-neighbour resample of an RGBA buffer from `sw×sh` to `tw×th`. Output
/// is exactly `tw*th*4` bytes. Cheap and dependency-free — enough for a
/// downscaled screencast; a real filter is a later refinement.
fn resample_nearest(src: &[u8], sw: u32, sh: u32, tw: u32, th: u32) -> Vec<u8> {
    let mut out = vec![0u8; (tw as usize) * (th as usize) * 4];
    for ty in 0..th {
        let sy = (ty * sh / th).min(sh.saturating_sub(1));
        for tx in 0..tw {
            let sx = (tx * sw / tw).min(sw.saturating_sub(1));
            let si = ((sy * sw + sx) as usize) * 4;
            let di = ((ty * tw + tx) as usize) * 4;
            if let (Some(s), Some(d)) = (src.get(si..si + 4), out.get_mut(di..di + 4)) {
                d.copy_from_slice(s);
            }
        }
    }
    out
}

/// Throttles a high-frequency frame source — the window's *present* clock,
/// delivered via `window::frames()` — down to the recording's target cadence
/// cadence. Driving the screencast off real presents (rather than a wall-clock
/// thread timer) is what keeps an idle window's screenshots resolving in real
/// time; this throttle then keeps only one frame per `interval`, so the GIF
/// plays back in real time instead of capturing every refresh.
///
/// Pure over a *logical* timeline: the caller supplies `elapsed` (monotonic time
/// since recording start, a plain [`Duration`]), so the real clock stays in the
/// adapter and the decision is exhaustively unit-testable with fake timers — no
/// `Instant::now()` in a test.
#[derive(Debug, Clone)]
pub struct FrameThrottle {
    interval: Duration,
    last: Option<Duration>,
}

impl FrameThrottle {
    #[must_use]
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: None,
        }
    }

    /// Whether the frame presented at `elapsed` (since recording start) should be
    /// captured: always for the first frame, then only once `interval` has passed
    /// since the last capture (spacing from the last *accepted* frame, so a stall
    /// never bursts a catch-up flurry). Advances the schedule on a hit.
    pub fn should_capture(&mut self, elapsed: Duration) -> bool {
        match self.last {
            // Inside the interval since the last accepted frame → skip.
            Some(last) if elapsed.saturating_sub(last) < self.interval => false,
            // First frame, or the interval has elapsed → capture and re-anchor.
            _ => {
                self.last = Some(elapsed);
                true
            }
        }
    }
}

/// Inter-arrival statistics for captured frames — the measurement harness for
/// the present-gating bug. On an idle window the gap between a screenshot
/// request and its result balloons far past the frame interval (the time-lapse
/// the issue reports); once presents are driven the gaps sit at ~the interval.
/// Logging this summary at stop turns "feels slow" into numbers, before and
/// after the fix.
///
/// A running accumulator (no per-frame allocation); the adapter feeds it the
/// `Instant`-derived gap for each delivered frame.
#[derive(Debug, Clone, Default)]
pub struct FrameStats {
    count: u32,
    sum: Duration,
    min: Option<Duration>,
    max: Option<Duration>,
}

/// A closed-out [`FrameStats`] summary: how many gaps were seen and their
/// spread. `None` from [`FrameStats::summary`] when no gap was recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameGapSummary {
    pub gaps: u32,
    pub min: Duration,
    pub max: Duration,
    pub mean: Duration,
}

impl FrameStats {
    /// Record one frame-delivery gap (time between consecutive delivered
    /// frames).
    pub fn record_gap(&mut self, gap: Duration) {
        self.count = self.count.saturating_add(1);
        self.sum = self.sum.saturating_add(gap);
        self.min = Some(self.min.map_or(gap, |m| m.min(gap)));
        self.max = Some(self.max.map_or(gap, |m| m.max(gap)));
    }

    /// The spread of the recorded gaps, or `None` when fewer than one gap was
    /// seen.
    #[must_use]
    pub fn summary(&self) -> Option<FrameGapSummary> {
        // `min`/`max` are `Some` exactly when `count >= 1`, so the divide is safe.
        let (min, max) = (self.min?, self.max?);
        Some(FrameGapSummary {
            gaps: self.count,
            min,
            max,
            mean: self.sum / self.count,
        })
    }
}

/// The GIF per-frame delay for a real-time gap: the wall-clock time the
/// frame was on screen, in centiseconds (the GIF time unit), floored at 1cs so
/// no frame is zero-duration and clamped to the `u16` ceiling. Using the *real*
/// gap — not a fixed `100/fps` — keeps playback real-time even when capture
/// jitters.
#[must_use]
fn gap_to_delay_cs(gap: Duration) -> u16 {
    let centiseconds = gap.as_millis() / 10;
    centiseconds.clamp(1, u128::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_budget_and_derived_values() {
        let c = RecordConfig::default();
        assert_eq!((c.fps, c.max_seconds), (8, 30));
        assert_eq!(c.max_frames(), 240);
        assert_eq!(c.frame_interval(), Duration::from_secs_f32(0.125));
    }

    #[test]
    fn target_dims_scales_and_floors_at_one() {
        assert_eq!(target_dims(800, 600, 0.5), (400, 300));
        assert_eq!(target_dims(1, 1, 0.5), (1, 1)); // never zero
    }

    #[test]
    fn resample_downscale_picks_nearest_source_pixels() {
        // A 2×2 image, one solid colour per pixel, downscaled to 1×1 picks the
        // top-left source pixel (sx=sy=0).
        let src = vec![
            10, 20, 30, 255, // (0,0)
            40, 50, 60, 255, // (1,0)
            70, 80, 90, 255, // (0,1)
            99, 99, 99, 255, // (1,1)
        ];
        let out = resample_nearest(&src, 2, 2, 1, 1);
        assert_eq!(out, vec![10, 20, 30, 255]);
    }

    #[test]
    fn resample_output_length_matches_target() {
        let src = vec![0u8; 4 * 4 * 4]; // 4×4 RGBA
        let out = resample_nearest(&src, 4, 4, 3, 2);
        assert_eq!(out.len(), 3 * 2 * 4);
    }

    // ---- real-time cadence — throttle the present clock to fps ----

    // Fake-timer helper: a logical millisecond offset since recording start. No
    // `Instant::now()` anywhere in these tests, so they are deterministic.
    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn the_first_presented_frame_is_always_captured() {
        let mut throttle = FrameThrottle::new(ms(125));
        assert!(
            throttle.should_capture(ms(0)),
            "the opening frame must be captured regardless of timing"
        );
    }

    #[test]
    fn a_frame_within_the_interval_is_skipped() {
        let mut throttle = FrameThrottle::new(ms(125));
        assert!(throttle.should_capture(ms(0)));
        // A refresh ~16ms later (60Hz) is far inside the 125ms interval → skip.
        assert!(
            !throttle.should_capture(ms(16)),
            "a refresh inside the interval must not be captured"
        );
    }

    #[test]
    fn a_frame_at_or_past_the_interval_is_captured_and_resets_the_clock() {
        let mut throttle = FrameThrottle::new(ms(125));
        assert!(throttle.should_capture(ms(0)));
        assert!(
            throttle.should_capture(ms(125)),
            "a frame at the interval captures"
        );
        // Spacing is measured from the last *accepted* frame, so a refresh just
        // after 125ms is skipped (not bursted to catch up).
        assert!(
            !throttle.should_capture(ms(141)),
            "spacing resets to the last captured frame"
        );
    }

    proptest::proptest! {
        /// Over a stream of refreshes spaced by `step`, the throttle never
        /// accepts two frames closer than `interval`, and always accepts the
        /// first — so the captured cadence is at most the configured fps and the
        /// GIF is real-time, not time-lapsed. The timeline is fake (logical ms),
        /// so the property is deterministic.
        #[test]
        fn captures_are_never_spaced_below_the_interval(
            interval_ms in 1u64..500,
            step_ms in 1u64..200,
            ticks in 1usize..300,
        ) {
            let interval = ms(interval_ms);
            let mut throttle = FrameThrottle::new(interval);
            let mut captures: Vec<Duration> = Vec::new();
            for i in 0..ticks {
                let elapsed = ms(step_ms * i as u64);
                if throttle.should_capture(elapsed) {
                    captures.push(elapsed);
                }
            }
            proptest::prop_assert!(!captures.is_empty(), "at least the first frame is captured");
            for pair in captures.windows(2) {
                proptest::prop_assert!(
                    pair[1] - pair[0] >= interval,
                    "two captures closer than the interval"
                );
            }
        }
    }

    // ---- the measurement harness (benchmark of the present-gating) ----

    #[test]
    fn frame_stats_has_no_summary_before_any_gap() {
        let stats = FrameStats::default();
        assert_eq!(stats.summary(), None, "no gap recorded → no spread");
    }

    #[test]
    fn frame_stats_accumulates_min_max_and_mean() {
        let mut stats = FrameStats::default();
        stats.record_gap(Duration::from_millis(100));
        stats.record_gap(Duration::from_millis(300));
        stats.record_gap(Duration::from_millis(200));
        let summary = stats.summary().expect("three gaps yield a summary");
        assert_eq!(summary.gaps, 3);
        assert_eq!(summary.min, Duration::from_millis(100));
        assert_eq!(summary.max, Duration::from_millis(300));
        assert_eq!(summary.mean, Duration::from_millis(200));
    }

    // ---- real-time GIF playback — per-frame delay from the real gap ----

    #[test]
    fn gap_delay_rounds_to_centiseconds() {
        // 125ms on screen ≈ 12cs (the GIF time unit is 1/100s).
        assert_eq!(gap_to_delay_cs(Duration::from_millis(125)), 12);
    }

    #[test]
    fn gap_delay_floors_at_one_centisecond() {
        // A sub-centisecond gap must never be zero-duration (a frozen frame).
        assert_eq!(gap_to_delay_cs(Duration::from_millis(2)), 1);
        assert_eq!(gap_to_delay_cs(Duration::ZERO), 1);
    }

    #[test]
    fn gap_delay_clamps_to_the_u16_ceiling() {
        // A multi-minute gap saturates rather than wrapping.
        assert_eq!(gap_to_delay_cs(Duration::from_secs(3600)), u16::MAX);
    }
}
