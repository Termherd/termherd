//! Permission bits, and what another user could plant first at a predictable
//! name in `/tmp`: both Unix notions, so the whole file is.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use termherd_private_fs::{ensure_dir, is_private, write_file};

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn a_created_file_is_owner_only() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("dump.png");
    write_file(&path, b"\x89PNG").expect("write");
    assert_eq!(mode(&path), 0o600);
    assert_eq!(std::fs::read(&path).expect("read"), b"\x89PNG");
}

#[test]
fn a_missing_directory_is_created_owner_only_with_its_parents() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("data").join("captures");
    ensure_dir(&path).expect("ensure");
    assert_eq!(mode(&path), 0o700);
}

#[test]
fn an_existing_world_readable_directory_is_tightened_not_refused() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("captures");
    std::fs::create_dir(&path).expect("create");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("open it up");
    ensure_dir(&path).expect("ensure");
    assert_eq!(mode(&path), 0o700);
}

#[test]
fn privacy_is_read_from_the_group_and_other_bits() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("file");
    std::fs::write(&path, "").expect("write");
    for (bits, private) in [(0o600, true), (0o700, true), (0o640, false), (0o604, false)] {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(bits)).expect("chmod");
        assert_eq!(is_private(&path).expect("stat"), private, "mode {bits:o}");
    }
}

#[test]
fn an_entry_that_cannot_be_removed_is_refused_not_written_through() {
    // A read-only directory stands in for a sticky `/tmp`: its entry cannot be
    // unlinked, yet can still be opened for writing.
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("locked");
    std::fs::create_dir(&dir).expect("the directory");
    let planted = dir.join("termherd-mcp-85.json");
    std::fs::write(&planted, "planted").expect("plant the file");
    std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o666))
        .expect("open it to everyone");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500))
        .expect("lock the directory");
    if std::fs::File::create(dir.join("probe")).is_ok() {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .expect("unlock for cleanup");
        eprintln!("skipped: root ignores the directory's mode");
        return;
    }

    let outcome = write_file(&planted, "token");
    let contents = std::fs::read_to_string(&planted).expect("readable");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .expect("unlock for cleanup");
    assert!(outcome.is_err(), "an entry we could not remove was adopted");
    assert_eq!(contents, "planted", "and the token was written through it");
}
