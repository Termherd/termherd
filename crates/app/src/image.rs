//! Pure RGBA pixel ops, shared by every reader of a window screenshot.
//!
//! Three readers now want the same two operations — fit a frame to a target
//! size, and encode it. The screencast recorder resamples to a scaled GIF
//! canvas, the capture dump writes a full-size PNG to disk, and the MCP
//! `screenshot` tool encodes a downscaled PNG into a tool result. Each of them
//! hand-rolling "how big should this be" or "how do I get PNG bytes" is exactly
//! the duplicated-invariant drift AGENTS.md warns about, so the arithmetic and
//! the encoder live here once.
//!
//! Everything in this module is a pure function of its arguments — no window,
//! no filesystem, no clock — so the sizing rules are exhaustively testable
//! without a GUI.

use std::io;

/// Output dimensions for a source frame scaled by `scale`, each at least 1
/// pixel and clamped to the GIF `u16` ceiling (the tightest of the callers).
#[must_use]
pub fn target_dims(sw: u32, sh: u32, scale: f32) -> (u32, u32) {
    let scaled = |n: u32| ((n as f32 * scale).round() as u32).clamp(1, u32::from(u16::MAX));
    (scaled(sw), scaled(sh))
}

/// Hard ceiling on the pixels an image carried in a tool result may have.
///
/// A width bound alone does not bound a payload: a portrait or rotated monitor
/// is tall, so a 1000×3000 window sails under a 1200-wide bound at three
/// megapixels — several MB of PNG, then a third more as base64, in the caller's
/// context. This ceiling is what actually holds the promise the width bound
/// only appears to make. Set above the common landscape case (1200×750) so the
/// default call is never silently shrunk twice.
pub const MAX_PAYLOAD_PIXELS: u64 = 1_440_000;

/// Output dimensions for a frame destined for a tool result: bounded to
/// `max_width`, then to [`MAX_PAYLOAD_PIXELS`], keeping the aspect ratio and
/// **never upscaling** — a frame already smaller than the bounds is returned
/// untouched, so asking for more width than the window has cannot inflate the
/// payload with interpolated pixels.
///
/// `None` for a frame with no pixels, which is the one case a caller must not
/// try to encode. Returning it here rather than re-testing the dimensions at
/// each call site keeps "is there an image to make?" a single predicate.
///
/// Floors at 1 pixel otherwise: a very wide, very short frame must not round
/// its height away to zero, which would make an empty image out of a real one.
#[must_use]
pub fn fit_payload(sw: u32, sh: u32, max_width: u32) -> Option<(u32, u32)> {
    if sw == 0 || sh == 0 {
        return None;
    }
    let (width, height) = if sw <= max_width {
        (sw, sh)
    } else {
        let height = ((u64::from(sh) * u64::from(max_width)) / u64::from(sw)) as u32;
        (max_width.max(1), height.max(1))
    };
    Some(under_pixel_ceiling(width, height))
}

/// Shrink `width`×`height` proportionally until it fits [`MAX_PAYLOAD_PIXELS`],
/// leaving anything already under it untouched. Both sides scale by the same
/// square root of the area ratio, so the aspect ratio survives.
fn under_pixel_ceiling(width: u32, height: u32) -> (u32, u32) {
    let pixels = u64::from(width) * u64::from(height);
    if pixels <= MAX_PAYLOAD_PIXELS {
        return (width, height);
    }
    let ratio = (MAX_PAYLOAD_PIXELS as f64 / pixels as f64).sqrt();
    let shrink = |n: u32| ((f64::from(n) * ratio) as u32).max(1);
    (shrink(width), shrink(height))
}

/// Resample an RGBA buffer from `sw×sh` to `tw×th`, picking the filter the
/// direction deserves. Output is exactly `tw*th*4` bytes.
///
/// **Shrinking averages** ([`resample_box`]); anything else takes the nearest
/// source pixel. The distinction is not cosmetic: a screenshot bound to 1200px
/// on a retina display is a ~0.4× reduction, and nearest-neighbour there
/// discards three pixel rows in five — terminal glyphs alias into noise, and
/// the image cannot answer the rendering question it was requested for.
/// Averaging costs one pass over the source and keeps text legible.
///
/// It also costs **payload**: measured on a real window at 900px, the averaged
/// PNG is ~40% larger than the nearest one (130 kB → 185 kB), because the
/// intermediate colours it creates compress worse than flat runs. The pixel
/// ceiling bounds area, not bytes, so that is a real increase per call. Worth
/// it — an illegible image costs its whole payload for nothing — but a caller
/// that only needs a coarse view should lower its width bound rather than
/// expect this to be cheap.
#[must_use]
pub fn resample(src: &[u8], sw: u32, sh: u32, tw: u32, th: u32) -> Vec<u8> {
    if tw <= sw && th <= sh {
        resample_box(src, sw, sh, tw, th)
    } else {
        resample_nearest(src, sw, sh, tw, th)
    }
}

/// Box-average resample: each output pixel is the mean of the source pixels it
/// covers. Only meaningful when shrinking — a growing axis maps each output
/// pixel to a sub-pixel box, which averages a single sample and degenerates to
/// nearest anyway, so [`resample`] routes those elsewhere.
///
/// Channels are averaged independently, alpha included: a window screenshot is
/// opaque, and averaging alpha with colour would be wrong on anything else.
#[must_use]
pub fn resample_box(src: &[u8], sw: u32, sh: u32, tw: u32, th: u32) -> Vec<u8> {
    let mut out = vec![0u8; (tw as usize) * (th as usize) * 4];
    if tw == 0 || th == 0 {
        return out;
    }
    for ty in 0..th {
        // The half-open source row span this output row covers, always at least
        // one row so no output pixel is left with an empty box to average.
        let y0 = ty * sh / th;
        let y1 = (((ty + 1) * sh / th).max(y0 + 1)).min(sh);
        for tx in 0..tw {
            let x0 = tx * sw / tw;
            let x1 = (((tx + 1) * sw / tw).max(x0 + 1)).min(sw);
            let mut sums = [0u64; 4];
            let mut count = 0u64;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let si = ((sy * sw + sx) as usize) * 4;
                    if let Some(pixel) = src.get(si..si + 4) {
                        for (sum, channel) in sums.iter_mut().zip(pixel) {
                            *sum += u64::from(*channel);
                        }
                        count += 1;
                    }
                }
            }
            let di = ((ty * tw + tx) as usize) * 4;
            if count > 0
                && let Some(d) = out.get_mut(di..di + 4)
            {
                for (channel, sum) in d.iter_mut().zip(sums) {
                    *channel = (sum / count) as u8;
                }
            }
        }
    }
    out
}

/// Nearest-neighbour resample of an RGBA buffer from `sw×sh` to `tw×th`. Output
/// is exactly `tw*th*4` bytes. The cheap filter, and the right one when a frame
/// grows rather than shrinks; [`resample`] picks between the two.
#[must_use]
pub fn resample_nearest(src: &[u8], sw: u32, sh: u32, tw: u32, th: u32) -> Vec<u8> {
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

/// Encode an RGBA buffer as PNG bytes. The one encoder: the capture dump writes
/// these bytes to a file, the MCP tool base64s them into a tool result.
pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(io::Error::other)?;
        writer.write_image_data(rgba).map_err(io::Error::other)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_dims_scales_and_floors_at_one() {
        assert_eq!(target_dims(800, 600, 0.5), (400, 300));
        assert_eq!(target_dims(1, 1, 0.5), (1, 1)); // never zero
    }

    #[test]
    fn fit_payload_keeps_the_aspect_ratio() {
        assert_eq!(fit_payload(3000, 2000, 1200), Some((1200, 800)));
    }

    #[test]
    fn fit_payload_never_upscales_a_smaller_frame() {
        // A window narrower than the bound is returned as-is: interpolated
        // pixels would cost payload without adding detail.
        assert_eq!(fit_payload(800, 600, 1200), Some((800, 600)));
    }

    #[test]
    fn fit_payload_floors_a_flat_frame_at_one_pixel_high() {
        // 4000×3 bounded to 100 would round the height to 0 — an empty image.
        assert_eq!(fit_payload(4000, 3, 100), Some((100, 1)));
    }

    #[test]
    fn a_frame_missing_either_dimension_has_no_fit() {
        // Either side alone makes the frame empty — a window mid-resize can
        // report one zero without the other.
        assert_eq!(fit_payload(0, 600, 1200), None, "no width, no image");
        assert_eq!(fit_payload(800, 0, 1200), None, "no height, no image");
        assert_eq!(fit_payload(0, 0, 1200), None);
    }

    #[test]
    fn a_tall_window_is_bounded_by_pixels_the_width_bound_never_reaches() {
        // A rotated monitor: 1000 wide sails under a 1200-wide bound, but three
        // megapixels is megabytes of base64 in the caller's context.
        let (w, h) = fit_payload(1000, 3000, 1200).expect("a fit");
        assert!(
            u64::from(w) * u64::from(h) <= MAX_PAYLOAD_PIXELS,
            "{w}×{h} is still over the ceiling"
        );
        // The aspect ratio survives the second shrink (1:3, within rounding).
        assert!((h as f64 / w as f64 - 3.0).abs() < 0.01, "{w}×{h}");
    }

    #[test]
    fn the_common_landscape_call_is_not_shrunk_twice() {
        // A retina 16:10 window at the default bound lands under the ceiling,
        // so the pixel rule must leave the width fit exactly as it found it.
        assert_eq!(fit_payload(2880, 1800, 1200), Some((1200, 750)));
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
    fn resample_picks_a_distinct_source_pixel_per_axis() {
        // The 1×1 case above cannot tell a right index from a wrong one: every
        // arithmetic slip still lands on pixel (0,0). This 4×4 → 2×2 grid gives
        // each source pixel a unique red channel (row*16 + col) and shrinks on
        // *both* axes, so a swapped axis, a dropped stride or an off-by-one row
        // shows up as a different value rather than the same one. Shrinking
        // vertically matters on its own: with equal source and target heights,
        // a wrong row index still clamps back onto the right row.
        let mut src = Vec::new();
        for row in 0..4u8 {
            for col in 0..4u8 {
                src.extend_from_slice(&[row * 16 + col, 0, 0, 255]);
            }
        }
        let out = resample_nearest(&src, 4, 4, 2, 2);
        let reds: Vec<u8> = out.chunks_exact(4).map(|px| px[0]).collect();
        // Columns 0 and 2 of rows 0 and 2: (0,0)=0, (0,2)=2, (2,0)=32, (2,2)=34.
        assert_eq!(reds, vec![0, 2, 32, 34]);
    }

    #[test]
    fn shrinking_averages_the_pixels_it_covers() {
        // The same 4×4 grid the nearest test uses, so the two can be compared
        // directly: nearest picks (0,0)=0, the box averages 0,1,16,17 → 8.
        let mut src = Vec::new();
        for row in 0..4u8 {
            for col in 0..4u8 {
                src.extend_from_slice(&[row * 16 + col, 0, 0, 255]);
            }
        }
        let reds: Vec<u8> = resample(&src, 4, 4, 2, 2)
            .chunks_exact(4)
            .map(|px| px[0])
            .collect();
        assert_eq!(reds, vec![8, 10, 40, 42], "each output is its box's mean");
        assert_ne!(
            reds,
            vec![0, 2, 32, 34],
            "and is not the nearest-neighbour pick"
        );
    }

    #[test]
    fn averaging_keeps_a_thin_line_visible_where_nearest_drops_it() {
        // Why the filter matters, in the smallest form of the real failure: a
        // one-pixel white row on black, downscaled 4:1. Nearest lands between
        // rows and returns pure black — the glyph stroke is gone. The box mean
        // keeps a grey trace, which is what makes text legible at 0.4×.
        const WHITE_ROW: usize = 1;
        let mut src = vec![0u8; 4 * 4 * 4];
        for col in 0..4 {
            let i = (WHITE_ROW * 4 + col) * 4;
            src[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
        assert_eq!(
            resample_nearest(&src, 4, 4, 1, 1)[0],
            0,
            "nearest samples row 0 and loses the line entirely"
        );
        assert!(
            resample(&src, 4, 4, 1, 1)[0] > 0,
            "averaging keeps the line as a grey trace"
        );
    }

    #[test]
    fn resample_leaves_an_unchanged_size_untouched() {
        let src: Vec<u8> = (0..(2 * 2 * 4) as u8).collect();
        assert_eq!(resample(&src, 2, 2, 2, 2), src, "a no-op stays a no-op");
    }

    #[test]
    fn a_growing_frame_falls_back_to_nearest() {
        // Averaging a sub-pixel box is meaningless; the routing must not send
        // an upscale down that path (and must still size the output right).
        let src = vec![10u8, 20, 30, 255];
        let out = resample(&src, 1, 1, 2, 2);
        assert_eq!(out.len(), 2 * 2 * 4);
        assert_eq!(&out[..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn resample_output_length_matches_target() {
        let src = vec![0u8; 4 * 4 * 4]; // 4×4 RGBA
        let out = resample_nearest(&src, 4, 4, 3, 2);
        assert_eq!(out.len(), 3 * 2 * 4);
    }

    #[test]
    fn encode_png_produces_bytes_that_decode_to_the_same_dimensions() {
        let rgba = vec![255u8, 0, 0, 255, 0, 255, 0, 255]; // 2×1
        let bytes = encode_png(&rgba, 2, 1).expect("encode");
        let reader = png::Decoder::new(bytes.as_slice())
            .read_info()
            .expect("decode");
        assert_eq!((reader.info().width, reader.info().height), (2, 1));
    }
}
