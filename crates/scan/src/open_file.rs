//! Opening a file the GUI thread reads, so that no file system object put in
//! its place can stall that thread or redirect the read.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Open `path` for reading without blocking on it. On Unix a FIFO would
/// otherwise park the open until a writer appears, and a symlink would be
/// followed; both are refused here, and the caller checks what it opened.
/// Windows has neither hazard on this path, so a plain open serves.
pub(crate) fn open_without_waiting(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_opened_without_waiting_for_a_writer() {
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = tempfile::tempdir().expect("tempdir");
        let fifo = dir.path().join("4242.json");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("run mkfifo");
        assert!(made.success(), "mkfifo failed");

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(open_without_waiting(&fifo).map(|file| file.metadata()));
        });
        let opened = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the open must return, not wait for a writer")
            .expect("open")
            .expect("metadata");
        assert!(
            !opened.file_type().is_file(),
            "the caller sees it is no file"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_not_followed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target.json");
        std::fs::write(&target, "{}").expect("write");
        let link = dir.path().join("4242.json");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        assert!(open_without_waiting(&link).is_err());
    }

    #[test]
    fn a_regular_file_opens() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("4242.json");
        std::fs::write(&path, "{}").expect("write");

        assert!(open_without_waiting(&path).is_ok());
    }
}
