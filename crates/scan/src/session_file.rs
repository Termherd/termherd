//! Reading Claude Code's per-process session files, `<dir>/<pid>.json` (the
//! directory is normally `~/.claude/sessions`). The decoding is the codec's;
//! this module owns only the bounds on the read.

use std::path::Path;

use termherd_claude::session_file::{self, SessionFile};

/// Far above the ~500 bytes Claude Code writes, far below what would stall the
/// GUI thread that reads it on every `snapshot`.
pub const MAX_SESSION_FILE_BYTES: u64 = 16 * 1024;

/// The session file of process `pid` under `dir`, decoded.
///
/// `None` for every way the file can fail to describe that process, none of
/// them an error to the caller: absent (an older CLI, a session still
/// starting), a symlink (the path is fixed by convention, so a link there is
/// something other than Claude Code's own write), larger than
/// [`MAX_SESSION_FILE_BYTES`], unreadable, or not decodable as `pid`'s file.
#[must_use]
pub fn read_session_file(dir: &Path, pid: u32) -> Option<SessionFile> {
    let _ = (dir, pid, session_file::parse);
    None
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
