//! What the walk may enter and read under the projects root. A leaf: `walk`
//! and `derive` both ask it, so the two can never disagree on which entry is
//! a folder, which is a transcript, or how much of one is read.

use std::fs::{self, FileType};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::open_file::open_without_waiting;

/// The most of one transcript the walk reads. Up to it a transcript is read
/// whole, and the largest seen on a working machine was 154 MB, so no real
/// file is cut. Past it the digest still gets what it uses: the summary, slug
/// and indexed text from the head, the last titles, model and tail snippets
/// from the end; only the message count misses the skipped middle.
pub(crate) const MAX_TRANSCRIPT_BYTES: u64 = 256 * 1024 * 1024;

/// The type of `path` itself, never of what a symlink there points to: a link
/// under the projects root points wherever its author chose, and what the
/// walk reads reaches the sidebar and the search index. A symlink is neither
/// a directory nor a file here.
fn own_type(path: &Path) -> Option<FileType> {
    fs::symlink_metadata(path).ok().map(|meta| meta.file_type())
}

/// Whether the walk descends into `path`.
pub(crate) fn is_dir(path: &Path) -> bool {
    own_type(path).is_some_and(|t| t.is_dir())
}

/// Whether the walk treats `path` as a file it may read.
pub(crate) fn is_file(path: &Path) -> bool {
    own_type(path).is_some_and(|t| t.is_file())
}

/// The text of the transcript at `path`, within [`MAX_TRANSCRIPT_BYTES`], or
/// `None` when it cannot be read.
pub(crate) fn read_transcript(path: &Path) -> Option<String> {
    read_bounded(path, MAX_TRANSCRIPT_BYTES)
}

/// `path` whole when it fits in `cap` bytes, else its first and last `cap / 2`
/// bytes with the line torn at each cut dropped, so the decoder sees only
/// whole lines. The type is checked on the descriptor: a symlink swapped in
/// after the walk's own check is refused at open on Unix.
fn read_bounded(path: &Path, cap: u64) -> Option<String> {
    let mut file = open_without_waiting(path).ok()?;
    let len = file
        .metadata()
        .ok()
        .filter(|meta| meta.file_type().is_file())?
        .len();
    let mut bytes = Vec::new();
    if len <= cap {
        // Still bounded: a live session may append between the stat and here.
        (&mut file).take(cap).read_to_end(&mut bytes).ok()?;
        return String::from_utf8(bytes).ok();
    }
    let half = cap / 2;
    (&mut file).take(half).read_to_end(&mut bytes).ok()?;
    bytes.truncate(bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1));
    // Starting one byte early tells whether the tail begins on a line.
    file.seek(SeekFrom::Start(len - half - 1)).ok()?;
    let mut tail = Vec::new();
    file.take(half + 1).read_to_end(&mut tail).ok()?;
    let start = tail
        .iter()
        .position(|&b| b == b'\n')
        .map_or(tail.len(), |i| i + 1);
    bytes.extend_from_slice(&tail[start..]);
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(names: &[&str]) -> String {
        names.iter().map(|n| format!("{n}\n")).collect()
    }

    #[test]
    fn a_plain_folder_and_file_are_entered_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.jsonl");
        fs::write(&file, "{}\n").unwrap();

        assert!(is_dir(dir.path()));
        assert!(!is_file(dir.path()));
        assert!(is_file(&file));
        assert!(!is_dir(&file));
        assert!(!is_file(&dir.path().join("absent.jsonl")));
    }

    #[test]
    fn a_transcript_at_the_cap_is_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jsonl");
        let body = lines(&["aaa", "bbb", "ccc"]);
        fs::write(&path, &body).unwrap();

        assert_eq!(read_bounded(&path, body.len() as u64), Some(body));
    }

    #[test]
    fn a_transcript_past_the_cap_keeps_its_whole_head_and_tail_lines() {
        // Six 4-byte lines; a 12-byte cap reads the first and last 6 bytes,
        // `l1a\nl2` and `a\nl6a\n`: the torn `l2` and `a` go.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jsonl");
        fs::write(&path, lines(&["l1a", "l2a", "l3a", "l4a", "l5a", "l6a"])).unwrap();

        assert_eq!(
            read_bounded(&path, 12),
            Some(lines(&["l1a", "l6a"])),
            "a torn line at either cut is dropped, the middle is skipped"
        );
    }

    #[test]
    fn the_cut_falls_on_a_line_end_exactly() {
        // Six 3-byte lines: both 6-byte halves start and end on a line
        // boundary, so no line is torn and none is dropped.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jsonl");
        fs::write(&path, lines(&["l1", "l2", "l3", "l4", "l5", "l6"])).unwrap();

        assert_eq!(
            read_bounded(&path, 12),
            Some(lines(&["l1", "l2", "l5", "l6"]))
        );
    }

    #[test]
    fn a_single_line_past_the_cap_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jsonl");
        fs::write(&path, "x".repeat(64)).unwrap();

        assert_eq!(read_bounded(&path, 16), Some(String::new()));
    }

    #[test]
    fn a_transcript_that_is_not_utf8_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jsonl");
        fs::write(&path, [0xff, 0xfe, b'\n']).unwrap();

        assert_eq!(read_bounded(&path, 1024), None);
    }

    #[test]
    fn an_absent_transcript_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_bounded(&dir.path().join("absent.jsonl"), 1024), None);
    }

    #[test]
    fn a_folder_in_place_of_a_transcript_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_bounded(dir.path(), 1024), None);
    }

    // The largest transcript seen on a working machine was 154 MB; the cap
    // leaves room above it so no real file loses a line.
    const _: () = assert!(MAX_TRANSCRIPT_BYTES > 154_000_000 * 3 / 2);
}
