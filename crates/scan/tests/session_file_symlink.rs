//! The symlink refusal needs `std::os::unix::fs::symlink`, an OS-only API, so
//! it lives here rather than in the adapter's own source: the OS-cfg
//! containment check scans `src/` only.
#![cfg(unix)]
#![allow(
    clippy::expect_used,
    reason = "a test binary; `allow-expect-in-tests` only covers #[cfg(test)] items"
)]

use std::fs;
use std::os::unix::fs::symlink;

use termherd_scan::read_session_file;

#[test]
fn a_symlink_in_place_of_the_file_is_not_followed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let elsewhere = dir.path().join("elsewhere.json");
    fs::write(&elsewhere, r#"{"pid":4242,"name":"planted"}"#).expect("write target");
    symlink(&elsewhere, dir.path().join("4242.json")).expect("symlink");

    assert_eq!(read_session_file(dir.path(), 4242), None);
}
