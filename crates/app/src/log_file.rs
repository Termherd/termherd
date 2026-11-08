//! The log file a release build on Windows writes its `tracing` output to.
//!
//! Its manifest asks Windows for no console when nothing hands it one (see
//! `build.rs`), so launched from Explorer or the Start menu its stderr goes
//! nowhere: the log needs a file. It lives in the data dir and is opened for
//! **append**, never truncated, because tracing
//! starts before the single-instance lock: a second launch that truncated it
//! would erase the running instance's log on its way to exiting.

use std::fs::File;
use std::path::{Path, PathBuf};

use crate::paths::DataDir;

/// The log's name in the data dir.
const NAME: &str = "termherd.log";

/// Past this size the log is set aside at startup, so it cannot grow forever.
pub const ROTATE_AT: u64 = 5 * 1024 * 1024;

/// Where this build logs: the file in `data_dir` for a release build on
/// Windows, `None` (stderr) for every other build.
///
/// The file even when a terminal launched it and lent it a console: one home
/// for the log of the build users install, wherever it was started. A debug
/// build is started from a terminal (`cargo run`), so it keeps stderr.
pub fn path(data_dir: Option<&DataDir>) -> Option<PathBuf> {
    if cfg!(all(windows, not(debug_assertions))) {
        data_dir.map(|data| data.file(NAME))
    } else {
        None
    }
}

/// Open `path` for appending, creating it (and the private data dir holding
/// it) if need be.
///
/// When it already holds `rotate_at` bytes or more, it is first renamed to
/// `<path>.old` (replacing an earlier one) and a fresh file is started. An
/// instance still writing to it keeps its handle, so its log carries on in
/// `.old`.
pub fn open(path: &Path, rotate_at: u64) -> std::io::Result<File> {
    if let Some(dir) = path.parent() {
        termherd_private_fs::ensure_dir(dir)?;
    }
    if std::fs::metadata(path).is_ok_and(|meta| meta.len() >= rotate_at) {
        // A rename another program blocks (holding a log open without delete
        // sharing) only postpones the rotation: appending beats logging nowhere.
        let _ = std::fs::rename(path, old_path(path));
    }
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
}

/// Where [`open`] sets an oversized log aside.
fn old_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::{old_path, open};
    use std::io::Write;

    #[test]
    fn a_new_log_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("termherd.log");

        writeln!(open(&path, 100).unwrap(), "first").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\n");
    }

    #[test]
    fn a_missing_data_dir_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".termherd").join("termherd.log");

        writeln!(open(&path, 100).unwrap(), "first launch").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first launch\n");
    }

    #[test]
    fn a_second_open_appends_rather_than_truncating() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("termherd.log");
        std::fs::write(&path, "running instance\n").unwrap();

        writeln!(open(&path, 100).unwrap(), "second launch").unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "running instance\nsecond launch\n"
        );
        assert!(!old_path(&path).exists());
    }

    #[test]
    fn a_log_at_the_limit_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("termherd.log");
        std::fs::write(&path, "0123456789").unwrap();
        std::fs::write(old_path(&path), "older still").unwrap();

        writeln!(open(&path, 10).unwrap(), "fresh").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fresh\n");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("termherd.log.old")).unwrap(),
            "0123456789"
        );
    }

    #[test]
    fn a_log_another_instance_holds_open_is_still_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("termherd.log");
        let mut running = open(&path, 100).unwrap();
        write!(running, "0123456789").unwrap();

        writeln!(open(&path, 10).unwrap(), "second launch").unwrap();
        writeln!(running, " still running").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second launch\n");
        assert_eq!(
            std::fs::read_to_string(old_path(&path)).unwrap(),
            "0123456789 still running\n"
        );
    }

    #[test]
    fn a_rotation_over_an_old_log_still_held_open_still_opens_a_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("termherd.log");
        let mut oldest = open(&path, 100).unwrap();
        write!(oldest, "0123456789").unwrap();
        let mut older = open(&path, 10).unwrap();
        write!(older, "0123456789").unwrap();

        writeln!(open(&path, 10).unwrap(), "newest").unwrap();
        writeln!(oldest, " oldest").unwrap();

        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .ends_with("newest\n")
        );
    }

    #[test]
    fn a_log_just_under_the_limit_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("termherd.log");
        std::fs::write(&path, "012345678").unwrap();

        writeln!(open(&path, 10).unwrap(), "more").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "012345678more\n");
    }

    #[test]
    fn an_unopenable_log_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let not_a_dir = dir.path().join("file");
        std::fs::write(&not_a_dir, "").unwrap();

        assert!(open(&not_a_dir.join("termherd.log"), 10).is_err());
    }
}
