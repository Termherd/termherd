//! Capture adapter — write the state dump + PNG screenshot for the AI dev loop
//! (G1). `core` builds the pure [`WorkspaceSnapshot`]; this module owns the I/O
//! it deliberately keeps out: the clock, the JSON encoding, the PNG encoding,
//! and the on-disk layout.
//!
//! Artefacts land in `~/.termherd/captures/` as `capture-<ts>.json` and
//! `capture-<ts>.png`, where `<ts>` is a UTC `YYYYMMDD-HHMMSS-mmm` stamp. An AI
//! assistant reads the latest by picking the highest-stamped pair — the names
//! sort chronologically.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use iced::window::Screenshot;
use termherd_core::WorkspaceSnapshot;

use crate::snapshot_dto::SnapshotDto;

/// `<data dir>/captures` — the capture output dir (PRD §7 app data dir).
#[must_use]
pub fn captures_dir(dir: &crate::paths::DataDir) -> PathBuf {
    dir.file("captures")
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

/// `dump` with the secrets its terminal text shows masked
/// ([`termherd_core::redact`]). Only the terminals: the rest of the dump is
/// termherd's own state, not what a program printed.
#[must_use]
pub fn redacted(mut dump: WorkspaceSnapshot) -> WorkspaceSnapshot {
    for text in dump.terminals.values_mut() {
        *text = termherd_core::redact::redact(text);
    }
    dump
}

/// The kinds of file a capture or a recording writes into the capture dir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Artefact {
    /// The rung-0 JSON state dump.
    Dump,
    /// The rung-1 PNG screenshot, the dump's companion.
    Screenshot,
    /// A GIF screencast.
    Screencast,
}

impl Artefact {
    /// The file extension, without its dot.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Dump => "json",
            Self::Screenshot => "png",
            Self::Screencast => "gif",
        }
    }
}

/// `dir/capture-<stamp>.<ext>`: where the `kind` artefact of a stamp lives.
#[must_use]
pub fn artefact_path(dir: &Path, stamp: &str, kind: Artefact) -> PathBuf {
    dir.join(format!("capture-{stamp}.{}", kind.extension()))
}

/// Write the JSON dump to `dir/capture-<stamp>.json`, returning the path
/// written. The companion PNG shares the stamp.
pub fn write_dump(dir: &Path, stamp: &str, dump: &WorkspaceSnapshot) -> io::Result<PathBuf> {
    let path = artefact_path(dir, stamp, Artefact::Dump);
    let json = to_json(dump).map_err(io::Error::other)?;
    termherd_private_fs::write_file(&path, json)?;
    Ok(path)
}

/// Write an iced [`Screenshot`]'s RGBA pixels to a PNG at `path`, full size —
/// a file pays for the whole picture where a tool result does not. The encoder
/// itself is shared with the MCP `screenshot` tool ([`crate::image`]), so both
/// readers produce the same bytes.
pub fn write_png(path: &Path, screenshot: &Screenshot) -> io::Result<()> {
    let bytes = crate::image::encode_png(
        &screenshot.rgba,
        screenshot.size.width,
        screenshot.size.height,
    )?;
    termherd_private_fs::write_file(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use termherd_core::{
        ClaudeIdentity, ConfigSummary, FocusRef, PaneSnapshot, ProjectSnapshot, SessionKind,
        SessionStatus, SidebarSnapshot, TabSnapshot,
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
                    declared: false,
                }],
            }),
            tabs: Some(vec![
                TabSnapshot {
                    active: false,
                    parked: false,
                    title: "proj".to_owned(),
                    status: Some(SessionStatus::Idle),
                    panes: vec![pane(3, SessionStatus::Idle)],
                },
                TabSnapshot {
                    active: true,
                    parked: false,
                    title: "repo".to_owned(),
                    status: Some(SessionStatus::Busy),
                    panes: vec![pane(6, SessionStatus::Idle), pane(7, SessionStatus::Busy)],
                },
                TabSnapshot {
                    active: false,
                    parked: true,
                    title: "long run".to_owned(),
                    status: Some(SessionStatus::Busy),
                    panes: vec![pane(8, SessionStatus::Busy)],
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
            identity: ClaudeIdentity::default(),
            color: None,
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
        assert_eq!(json["tabs"][0]["title"], "proj");
        assert_eq!(json["tabs"][0]["status"], "idle");
        assert_eq!(json["tabs"][1]["panes"][1]["handle"], "7");
        assert_eq!(json["tabs"][1]["panes"][1]["status"], "busy");
        assert_eq!(json["tabs"][1]["parked"], false);
        assert_eq!(json["tabs"][2]["parked"], true);
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
        assert!(read.contains("\"repo\""), "dump should hold the tab title");
    }

    #[test]
    fn the_dump_and_the_screenshot_are_owner_only() {
        // The dump holds a whole terminal's text, the PNG a picture of it:
        // neither may be readable by another user of the machine.
        let dir = tempfile::tempdir().expect("tempdir");
        let dump = write_dump(dir.path(), "20010909-014640-000", &dump()).expect("write");
        let png = artefact_path(dir.path(), "20010909-014640-000", Artefact::Screenshot);
        let screenshot = Screenshot::new(
            crate::image::testing::numbered_frame(2, 1),
            iced::Size::new(2, 1),
            1.0,
        );
        write_png(&png, &screenshot).expect("write png");

        for path in [dump, png] {
            assert!(
                termherd_private_fs::is_private(&path).expect("stat"),
                "{} is readable by others",
                path.display()
            );
        }
    }

    #[test]
    fn write_png_round_trips_dimensions() {
        let screenshot = Screenshot::new(
            crate::image::testing::numbered_frame(2, 1),
            iced::Size::new(2, 1),
            1.0,
        );
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("shot.png");
        write_png(&path, &screenshot).expect("write png");

        let decoder = png::Decoder::new(std::fs::File::open(&path).expect("open"));
        let reader = decoder.read_info().expect("read info");
        assert_eq!((reader.info().width, reader.info().height), (2, 1));
    }
}
