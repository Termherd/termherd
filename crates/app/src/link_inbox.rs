//! The inbox a second launch drops a `termherd://` link into, for the
//! instance that holds the lock to pick up. A cold start reads it the same
//! way, so a link takes one path whichever process received it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::warn;

use crate::paths::DataDir;

/// A link longer than this is not one termherd wrote: it is dropped unread.
const MAX_LINK_BYTES: u64 = 16 * 1024;

const ENTRY: &str = ".url";

/// `<data dir>/inbox`.
#[must_use]
pub fn inbox_dir(data: &DataDir) -> PathBuf {
    data.file("inbox")
}

/// Drop `url` into `dir`, creating it when absent. The entry appears whole
/// or not at all: a reader never sees a half-written link.
///
/// Entries are named by arrival time, so their names sort in arrival order.
/// The final name is taken by a hard link, which fails rather than replace
/// an entry another launch wrote in the same nanosecond.
///
/// # Errors
/// The directory or the entry could not be written.
pub fn deposit(dir: &Path, url: &str) -> io::Result<PathBuf> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    deposit_at(dir, url, stamp)
}

fn deposit_at(dir: &Path, url: &str, stamp: u128) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let pid = std::process::id();
    let draft = dir.join(format!("{stamp:020}-{pid}{ENTRY}.tmp"));
    fs::write(&draft, url)?;
    let taken = (0..1000u128).find_map(|bump| {
        let entry = dir.join(format!("{:020}-{pid}{ENTRY}", stamp + bump));
        match fs::hard_link(&draft, &entry) {
            Ok(()) => Some(Ok(entry)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => None,
            Err(error) => Some(Err(error)),
        }
    });
    let _ = fs::remove_file(&draft);
    taken.unwrap_or_else(|| Err(io::Error::other("no free inbox entry name")))
}

/// Take every link waiting in `dir`, oldest first, removing each one read.
/// An entry that cannot be read as a link is removed and skipped.
#[must_use]
pub fn drain(dir: &Path) -> Vec<String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            warn!(%error, dir = %dir.display(), "link inbox unreadable");
            return Vec::new();
        }
    };
    let mut waiting: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.to_string_lossy().ends_with(ENTRY))
        .collect();
    waiting.sort();
    waiting.into_iter().filter_map(|path| take(&path)).collect()
}

/// Read one entry and remove it. A link whose entry cannot be removed is not
/// returned, so it is never acted on twice.
fn take(path: &Path) -> Option<String> {
    let read = read_link(path);
    if let Err(error) = fs::remove_file(path) {
        warn!(%error, entry = %path.display(), "link inbox entry not removed; skipped");
        return None;
    }
    read.inspect_err(|error| warn!(%error, entry = %path.display(), "link inbox entry dropped"))
        .ok()
}

fn read_link(path: &Path) -> io::Result<String> {
    if fs::metadata(path)?.len() > MAX_LINK_BYTES {
        return Err(io::Error::other("entry too long for a link"));
    }
    String::from_utf8(fs::read(path)?).map_err(|_| io::Error::other("entry is not UTF-8"))
}

/// The link an OS hands a launch on its command line (Windows, Linux).
#[must_use]
pub fn url_argument(args: impl IntoIterator<Item = String>) -> Option<String> {
    args.into_iter()
        .skip(1)
        .find(|arg| termherd_core::deeplink::is_link(arg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbox() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().expect("tempdir");
        let dir = root.path().join("inbox");
        (root, dir)
    }

    #[test]
    fn a_deposited_link_is_drained_once() {
        let (_root, dir) = inbox();
        deposit(&dir, "termherd://open?repo=/r").expect("deposit");

        assert_eq!(drain(&dir), ["termherd://open?repo=/r"]);
        assert!(drain(&dir).is_empty(), "a drained link is gone");
    }

    #[test]
    fn links_drain_in_the_order_they_arrived() {
        let (_root, dir) = inbox();
        for repo in ["/a", "/b", "/c"] {
            deposit(&dir, &format!("termherd://open?repo={repo}")).expect("deposit");
        }

        assert_eq!(
            drain(&dir),
            [
                "termherd://open?repo=/a",
                "termherd://open?repo=/b",
                "termherd://open?repo=/c",
            ]
        );
    }

    #[test]
    fn an_entry_still_being_written_is_left_alone() {
        let (_root, dir) = inbox();
        std::fs::create_dir_all(&dir).expect("mkdir");
        let in_flight = dir.join("00000000000000000001-1-0.url.tmp");
        std::fs::write(&in_flight, "termherd://open?repo=/r").expect("write");
        deposit(&dir, "termherd://open?repo=/done").expect("deposit");

        assert_eq!(drain(&dir), ["termherd://open?repo=/done"]);
        assert!(in_flight.exists(), "the writer still owns it");
    }

    #[test]
    fn an_unreadable_entry_is_dropped_not_retried() {
        let (_root, dir) = inbox();
        std::fs::create_dir_all(&dir).expect("mkdir");
        let garbled = dir.join("00000000000000000001-1-0.url");
        std::fs::write(&garbled, [0xFF, 0xFE, 0x00]).expect("write");

        assert!(drain(&dir).is_empty());
        assert!(!garbled.exists(), "a bad entry must not be read again");
    }

    #[test]
    fn a_missing_inbox_has_nothing_waiting() {
        let (_root, dir) = inbox();
        assert!(drain(&dir).is_empty());
    }

    #[test]
    fn two_links_deposited_in_the_same_instant_both_arrive_in_order() {
        let (_root, dir) = inbox();
        deposit_at(&dir, "termherd://open?repo=/a", 7).expect("deposit");
        deposit_at(&dir, "termherd://open?repo=/b", 7).expect("deposit");

        assert_eq!(
            drain(&dir),
            ["termherd://open?repo=/a", "termherd://open?repo=/b"]
        );
    }

    #[test]
    fn a_link_is_read_up_to_its_size_bound_and_no_further() {
        let (_root, dir) = inbox();
        let bound = usize::try_from(MAX_LINK_BYTES).expect("fits");
        let at_bound = "x".repeat(bound);
        deposit_at(&dir, &at_bound, 1).expect("deposit");
        deposit_at(&dir, &"x".repeat(bound + 1), 2).expect("deposit");

        assert_eq!(drain(&dir), [at_bound]);
        assert_eq!(MAX_LINK_BYTES, 16 * 1024);
    }

    #[test]
    fn a_deposit_creates_the_inbox() {
        let (_root, dir) = inbox();
        let entry = deposit(&dir, "termherd://open?repo=/r").expect("deposit");
        assert!(entry.starts_with(&dir));
    }

    #[test]
    fn the_os_hands_the_link_as_a_command_line_argument() {
        let args = |list: &[&str]| list.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();

        assert_eq!(url_argument(args(&["termherd"])), None);
        assert_eq!(
            url_argument(args(&["termherd", "termherd://open?repo=/r"])),
            Some("termherd://open?repo=/r".to_owned())
        );
        assert_eq!(
            url_argument(args(&["termherd", "TERMHERD://open?repo=/r"])),
            Some("TERMHERD://open?repo=/r".to_owned()),
            "the scheme is case-insensitive"
        );
        assert_eq!(
            url_argument(args(&["termherd", "--verbose"])),
            None,
            "anything else on the line is not a link"
        );
        assert_eq!(
            url_argument(args(&["termherd://open?repo=/r"])),
            None,
            "the first argument is the program, never a link"
        );
    }
}
