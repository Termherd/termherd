//! Reading Claude Code's per-process session files, `<dir>/<pid>.json` (the
//! directory is normally `~/.claude/sessions`). The decoding is the codec's;
//! this module owns only the bounds on the read.

use std::io::{self, Read};
use std::path::Path;

use crate::open_file::open_without_waiting;

use termherd_claude::session_file::{self, SessionFile};
use tracing::debug;

/// Far above the ~500 bytes Claude Code writes, far below what would stall the
/// GUI thread that reads it on every `snapshot`.
pub const MAX_SESSION_FILE_BYTES: u64 = 16 * 1024;

/// The session file of process `pid` under `dir`, decoded.
///
/// `None` for every way the file can fail to describe that process, none of
/// them an error to the caller: absent (an older CLI, a session still
/// starting), not a regular file, a symlink on Unix (the path is fixed by
/// convention, so a link there is something other than Claude Code's own
/// write), larger than [`MAX_SESSION_FILE_BYTES`], unreadable, or not
/// decodable as `pid`'s file.
#[must_use]
pub fn read_session_file(dir: &Path, pid: u32) -> Option<SessionFile> {
    let path = dir.join(format!("{pid}.json"));
    let file = open_without_waiting(&path)
        .inspect_err(|error| {
            // Absent is the common case (a shell's job, an older CLI).
            if error.kind() != io::ErrorKind::NotFound {
                debug!(path = %path.display(), %error, "session file unopenable");
            }
        })
        .ok()?;
    // Checked on the descriptor, not the path, so nothing swapped in after
    // the check can be what gets read.
    let opened = file.metadata().ok()?;
    if !opened.file_type().is_file() || opened.len() > MAX_SESSION_FILE_BYTES {
        debug!(path = %path.display(), "session file refused: not a regular file within bounds");
        return None;
    }
    let mut text = String::new();
    file.take(MAX_SESSION_FILE_BYTES)
        .read_to_string(&mut text)
        .inspect_err(|error| debug!(path = %path.display(), %error, "session file unreadable"))
        .ok()?;
    session_file::parse(&text, pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, pid: u32, body: &str) {
        fs::write(dir.join(format!("{pid}.json")), body).expect("write session file");
    }

    #[test]
    fn a_present_file_is_read_and_decoded() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            4242,
            r#"{"pid":4242,"name":"knowledge-hub-35","sessionId":"abc"}"#,
        );
        assert_eq!(
            read_session_file(dir.path(), 4242),
            Some(SessionFile {
                pid: 4242,
                name: Some("knowledge-hub-35".to_owned()),
                session_id: Some("abc".to_owned()),
                proc_start: None,
            })
        );
    }

    #[test]
    fn an_absent_file_is_no_identity_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(read_session_file(dir.path(), 4242), None);
    }

    #[test]
    fn a_file_over_the_bound_is_not_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let padding = " ".repeat(usize::try_from(MAX_SESSION_FILE_BYTES).expect("fits"));
        write(
            dir.path(),
            4242,
            &format!(r#"{{"pid":4242,"name":"big"{padding}}}"#),
        );
        assert_eq!(read_session_file(dir.path(), 4242), None);
    }

    #[test]
    fn a_file_over_the_bound_is_refused_even_when_its_head_is_a_whole_file() {
        // What `take` would hand the decoder is a complete object here, so only
        // the size check stands between it and a file of any length.
        let dir = tempfile::tempdir().expect("tempdir");
        let padding = " ".repeat(usize::try_from(MAX_SESSION_FILE_BYTES).expect("fits"));
        write(
            dir.path(),
            4242,
            &format!(r#"{{"pid":4242,"name":"big"}}{padding}"#),
        );
        assert_eq!(read_session_file(dir.path(), 4242), None);
    }

    #[test]
    fn a_file_at_the_bound_is_still_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let head = r#"{"pid":4242,"name":"edge""#;
        let fill = usize::try_from(MAX_SESSION_FILE_BYTES).expect("fits") - head.len() - 1;
        write(dir.path(), 4242, &format!("{head}{}}}", " ".repeat(fill)));
        assert_eq!(
            read_session_file(dir.path(), 4242).and_then(|file| file.name),
            Some("edge".to_owned())
        );
    }

    #[test]
    fn a_file_with_a_long_rename_history_is_still_read() {
        // Claude Code keeps every former name in the file, so a session renamed
        // often outgrows the ~500 bytes of a fresh one by several times.
        let dir = tempfile::tempdir().expect("tempdir");
        let former: Vec<String> = (0..200).map(|n| format!("\"proj-{n:03}\"")).collect();
        let body = format!(
            r#"{{"pid":4242,"name":"proj-200","formerNames":[{}]}}"#,
            former.join(",")
        );
        assert!(
            body.len() > 2 * 1024,
            "the fixture must be a large real-shaped file"
        );
        write(dir.path(), 4242, &body);
        assert_eq!(
            read_session_file(dir.path(), 4242).and_then(|file| file.name),
            Some("proj-200".to_owned())
        );
    }

    #[test]
    fn a_file_naming_another_process_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(dir.path(), 4242, r#"{"pid":4343,"name":"someone-else"}"#);
        assert_eq!(read_session_file(dir.path(), 4242), None);
    }

    #[test]
    fn a_directory_in_place_of_the_file_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("4242.json")).expect("mkdir");
        assert_eq!(read_session_file(dir.path(), 4242), None);
    }
}
