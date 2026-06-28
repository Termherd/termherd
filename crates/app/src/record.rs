//! Record adapter — the GIF encoder side of the screencast (#124, F-capture
//! rung 2). `core` owns the idle→recording state machine; this module owns the
//! I/O it keeps out: the frame buffer, the resample, the `gif` encoder, and the
//! file.
//!
//! Encoding (a NeuQuant palette reduction per frame) is CPU-heavy, so it runs on
//! a **dedicated recorder thread** fed over a channel — the UI thread only hands
//! off raw frames. Keeping the GUI free also keeps the *recording* smooth, since
//! a stalled UI would be captured in its own frames.
//!
//! Output: `~/.termherd/captures/capture-<ts>.gif`, the #108 capture dir.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use tracing::{info, warn};

/// Recording budget — frames per second, hard duration cap, and the scale the
/// captured frames are downsampled to. The default (8 fps / 30 s / 0.5×) keeps
/// GIFs manageable while smooth enough for a bug repro; making it configurable
/// via `settings.json` is a follow-up (#124).
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

    /// The wall-clock gap between frames — the app's record timer interval.
    #[must_use]
    pub fn frame_interval(&self) -> Duration {
        Duration::from_secs_f32(1.0 / self.fps.max(1) as f32)
    }

    /// Per-frame delay in centiseconds (the GIF time unit), so a player replays
    /// at the recorded fps. Floored at 1cs so no frame is zero-duration.
    fn delay_cs(&self) -> u16 {
        (100 / self.fps.max(1)).clamp(1, u32::from(u16::MAX)) as u16
    }
}

/// A frame or lifecycle signal sent to the recorder thread.
enum RecordMsg {
    /// One captured frame: raw RGBA and its physical pixel size.
    Frame {
        rgba: Vec<u8>,
        width: u32,
        height: u32,
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

    /// Hand one captured frame (raw RGBA + physical size) to the encoder thread.
    pub fn frame(&self, rgba: Vec<u8>, width: u32, height: u32) {
        let _ = self.tx.send(RecordMsg::Frame {
            rgba,
            width,
            height,
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
            } => {
                if session.is_none() {
                    let (tw, th) = target_dims(width, height, config.scale);
                    match Session::open(path, tw, th, config.delay_cs()) {
                        Ok(opened) => session = Some(opened),
                        Err(error) => {
                            warn!(%error, "could not open gif encoder; recording dropped");
                            return;
                        }
                    }
                }
                if let Some(session) = session.as_mut()
                    && let Err(error) = session.write(&rgba, width, height)
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
/// fit, not corrupted) and the per-frame delay.
struct Session {
    encoder: gif::Encoder<BufWriter<File>>,
    target: (u32, u32),
    delay_cs: u16,
}

impl Session {
    fn open(path: &PathBuf, tw: u32, th: u32, delay_cs: u16) -> std::io::Result<Self> {
        let writer = BufWriter::new(File::create(path)?);
        let mut encoder =
            gif::Encoder::new(writer, tw as u16, th as u16, &[]).map_err(std::io::Error::other)?;
        encoder
            .set_repeat(gif::Repeat::Infinite)
            .map_err(std::io::Error::other)?;
        Ok(Self {
            encoder,
            target: (tw, th),
            delay_cs,
        })
    }

    fn write(&mut self, rgba: &[u8], sw: u32, sh: u32) -> std::io::Result<()> {
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
        frame.delay = self.delay_cs;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_budget_and_derived_values() {
        let c = RecordConfig::default();
        assert_eq!((c.fps, c.max_seconds), (8, 30));
        assert_eq!(c.max_frames(), 240);
        assert_eq!(c.delay_cs(), 12); // 100/8 = 12cs ≈ 8fps
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
}
