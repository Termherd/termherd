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

/// Output dimensions for a source frame bounded to `max_width`, keeping the
/// aspect ratio and **never upscaling** — a frame already narrower than the
/// bound is returned untouched, so asking for more width than the window has
/// cannot inflate the payload with interpolated pixels.
///
/// `None` for a frame with no pixels, which is the one case a caller must not
/// try to encode. Returning it here rather than re-testing the dimensions at
/// each call site keeps "is there an image to make?" a single predicate.
///
/// Floors at 1 pixel otherwise: a very wide, very short frame must not round
/// its height away to zero, which would make an empty image out of a real one.
#[must_use]
pub fn fit_width(sw: u32, sh: u32, max_width: u32) -> Option<(u32, u32)> {
    if sw == 0 || sh == 0 {
        return None;
    }
    if sw <= max_width {
        return Some((sw, sh));
    }
    let height = ((u64::from(sh) * u64::from(max_width)) / u64::from(sw)) as u32;
    Some((max_width.max(1), height.max(1)))
}

/// Nearest-neighbour resample of an RGBA buffer from `sw×sh` to `tw×th`. Output
/// is exactly `tw*th*4` bytes. Cheap and dependency-free — enough for a
/// downscaled screencast or a screenshot an agent reads; a real filter is a
/// later refinement.
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
    fn fit_width_keeps_the_aspect_ratio() {
        assert_eq!(fit_width(3000, 2000, 1200), Some((1200, 800)));
    }

    #[test]
    fn fit_width_never_upscales_a_smaller_frame() {
        // A window narrower than the bound is returned as-is: interpolated
        // pixels would cost payload without adding detail.
        assert_eq!(fit_width(800, 600, 1200), Some((800, 600)));
    }

    #[test]
    fn fit_width_floors_a_flat_frame_at_one_pixel_high() {
        // 4000×3 bounded to 100 would round the height to 0 — an empty image.
        assert_eq!(fit_width(4000, 3, 100), Some((100, 1)));
    }

    #[test]
    fn a_frame_missing_either_dimension_has_no_fit() {
        // Either side alone makes the frame empty — a window mid-resize can
        // report one zero without the other.
        assert_eq!(fit_width(0, 600, 1200), None, "no width, no image");
        assert_eq!(fit_width(800, 0, 1200), None, "no height, no image");
        assert_eq!(fit_width(0, 0, 1200), None);
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
