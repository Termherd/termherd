//! Capture adapter — write the state dump + PNG screenshot for the AI dev loop
//! (G1). `core` builds the pure [`WorkspaceSnapshot`]; this module owns the I/O
//! it deliberately keeps out: the clock, the JSON encoding, the PNG encoding,
//! and the on-disk layout.
//!
//! Artefacts land in `~/.termherd/captures/` as `capture-<ts>.json` and
//! `capture-<ts>.png`, where `<ts>` is a UTC `YYYYMMDD-HHMMSS-mmm` stamp. An AI
//! assistant reads the latest by picking the highest-stamped pair — the names
//! sort chronologically.

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use iced::window::Screenshot;
use termherd_core::WorkspaceSnapshot;

use crate::snapshot_dto::SnapshotDto;

/// `~/.termherd/captures` — the capture output dir (PRD §7 app data dir). `None`
/// when no home directory is set, in which case capture is skipped.
#[must_use]
pub fn captures_dir() -> Option<PathBuf> {
    Some(crate::paths::termherd_dir()?.join("captures"))
}

/// A UTC `YYYYMMDD-HHMMSS-mmm` stamp for `now`, used as the capture filename
/// stem. Chronological string order matches time order, so the newest capture
/// is the lexicographically greatest. Falls back to the epoch for a clock set
/// before 1970 (never panics).
#[must_use]
pub fn stamp(now: SystemTime) -> String {
    let since = now.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    let secs = since.as_secs();
    let millis = since.subsec_millis();
    let (year, month, day) = civil_from_days((secs / 86_400) as i64);
    let secs_of_day = secs % 86_400;
    let (hour, minute, second) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}-{millis:03}")
}

/// Civil date (year, month, day) from a count of days since the Unix epoch,
/// after Howard Hinnant's `civil_from_days`. Pure integer arithmetic — no
/// calendar dependency, no panic.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // day-of-era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day-of-year [0, 365]
    let mp = (5 * doy + 2) / 153; // month-pivot [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Encode a [`WorkspaceSnapshot`] as pretty JSON — the same wire form the MCP
/// `snapshot` tool reports, so a reader learns one shape for both.
pub fn to_json(dump: &WorkspaceSnapshot) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&SnapshotDto::from(dump))
}

/// Write the JSON dump to `dir/capture-<stamp>.json`, returning the path
/// written. The companion PNG shares the stamp ([`png_path`]).
pub fn write_dump(dir: &Path, stamp: &str, dump: &WorkspaceSnapshot) -> io::Result<PathBuf> {
    let path = dir.join(format!("capture-{stamp}.json"));
    let json = to_json(dump).map_err(io::Error::other)?;
    std::fs::write(&path, json)?;
    Ok(path)
}

/// The PNG path for a stamp — the rung-1 companion of [`write_dump`]'s JSON.
#[must_use]
pub fn png_path(dir: &Path, stamp: &str) -> PathBuf {
    dir.join(format!("capture-{stamp}.png"))
}

/// Encode an iced [`Screenshot`]'s RGBA pixels to a PNG at `path`. The `png`
/// crate is already a dependency (window-icon decode), so this adds none.
pub fn write_png(path: &Path, screenshot: &Screenshot) -> io::Result<()> {
    let writer = BufWriter::new(File::create(path)?);
    let mut encoder = png::Encoder::new(writer, screenshot.size.width, screenshot.size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer
        .write_image_data(&screenshot.rgba)
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use termherd_core::{
        ConfigSummary, FocusRef, PaneSnapshot, ProjectSnapshot, SessionKind, SessionStatus,
        SidebarSnapshot, TabSnapshot,
    };

    #[test]
    fn stamp_formats_a_known_instant_in_utc() {
        // 1_000_000_000s since the epoch is 2001-09-09 01:46:40 UTC.
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        assert_eq!(stamp(now), "20010909-014640-000");
        // The epoch itself, with sub-second millis preserved.
        let epoch = UNIX_EPOCH + Duration::from_millis(431);
        assert_eq!(stamp(epoch), "19700101-000000-431");
    }

    #[test]
    fn stamps_sort_chronologically() {
        let earlier = stamp(UNIX_EPOCH + Duration::from_secs(1_000_000_000));
        let later = stamp(UNIX_EPOCH + Duration::from_secs(1_000_000_001));
        assert!(earlier < later, "{earlier} should sort before {later}");
    }

    fn dump() -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            focus: FocusRef {
                tab: Some(1),
                session: Some(7),
            },
            config: Some(ConfigSummary {
                font_size: 14.0,
                terminal_scheme: Some("gruvbox-dark".to_owned()),
                record_fps: 8,
                record_scale: 0.5,
                keymap_overrides: 2,
            }),
            sidebar: Some(SidebarSnapshot {
                hidden: false,
                search: String::new(),
                search_titles_only: false,
                show_archived: false,
                projects: vec![ProjectSnapshot {
                    path: "/proj".to_owned(),
                    session_count: 2,
                    collapsed: false,
                }],
            }),
            tabs: Some(vec![
                TabSnapshot {
                    active: false,
                    title: "proj $".to_owned(),
                    status: Some(SessionStatus::Idle),
                    panes: vec![pane(3, SessionStatus::Idle)],
                },
                TabSnapshot {
                    active: true,
                    title: "repo 🤖".to_owned(),
                    status: Some(SessionStatus::Busy),
                    panes: vec![pane(6, SessionStatus::Idle), pane(7, SessionStatus::Busy)],
                },
            ]),
            terminals: BTreeMap::from([(7, "$ cargo test".to_owned())]),
        }
    }

    fn pane(handle: u64, status: SessionStatus) -> PaneSnapshot {
        PaneSnapshot {
            handle,
            kind: SessionKind::Shell,
            cwd: Some("/proj".to_owned()),
            status,
        }
    }

    #[test]
    fn to_json_encodes_the_whole_workspace_snapshot() {
        let json: serde_json::Value =
            serde_json::from_str(&to_json(&dump()).expect("encode")).expect("valid json");
        // The dump is the *whole* app: focus, config, sidebar, tabs, terminals —
        // the same shape (and vocabulary) the MCP `snapshot` tool reports.
        assert_eq!(json["focus"]["tab"], 1);
        assert_eq!(json["focus"]["session"], "7", "handles are strings");
        assert_eq!(json["config"]["terminal_scheme"], "gruvbox-dark");
        assert_eq!(json["sidebar"]["projects"][0]["path"], "/proj");
        assert_eq!(json["tabs"][0]["title"], "proj $");
        assert_eq!(json["tabs"][0]["status"], "idle");
        assert_eq!(json["tabs"][1]["panes"][1]["handle"], "7");
        assert_eq!(json["tabs"][1]["panes"][1]["status"], "busy");
        assert_eq!(json["terminals"]["7"], "$ cargo test");
    }

    #[test]
    fn write_dump_writes_a_stamped_json_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_dump(dir.path(), "20010909-014640-000", &dump()).expect("write");
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("capture-20010909-014640-000.json")
        );
        let read = std::fs::read_to_string(&path).expect("read back");
        assert!(
            read.contains("\"repo 🤖\""),
            "dump should hold the tab title"
        );
    }

    #[test]
    fn write_png_round_trips_dimensions() {
        // A 2x1 RGBA image: two opaque pixels.
        let screenshot = Screenshot::new(
            vec![255u8, 0, 0, 255, 0, 255, 0, 255],
            iced::Size::new(2, 1),
            1.0,
        );
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("shot.png");
        write_png(&path, &screenshot).expect("write png");

        let decoder = png::Decoder::new(File::open(&path).expect("open"));
        let reader = decoder.read_info().expect("read info");
        assert_eq!((reader.info().width, reader.info().height), (2, 1));
    }
}
