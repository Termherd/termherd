//! The permission contract on the files that carry a bearer token.
//!
//! Two callers write one: a Claude launch's `--mcp-config`, and the attach
//! rung's published bridge file. Both hand whoever reads them the ability to
//! drive the workspace — `run_in_session` types into a terminal — so a
//! world-readable one would give that to every other account on the machine.
//!
//! It lives here rather than beside the function because the mode API is
//! Unix-only: an integration test may use `std::os::unix` freely, where the
//! same `#[cfg(unix)]` inside `src/` would need a quarantine entry in
//! `scripts/check-os-cfg-containment.sh` — and that entry would then licence
//! OS-conditional *production* code in the same file, unnoticed.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;

/// A fresh file is owner-only.
#[test]
fn a_private_file_is_readable_only_by_its_owner() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("secret.json");

    termherd_pty::write_private(&path, "{\"token\":\"hunter2\"}").expect("write");

    let mode = std::fs::metadata(&path)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "got {:o}", mode & 0o777);
    assert_eq!(
        std::fs::read_to_string(&path).expect("read back"),
        "{\"token\":\"hunter2\"}"
    );
}

/// Writing over a wider file does **not** narrow it — the mode is applied on
/// creation only. Callers that must guarantee the mode remove the file first;
/// pinning the behaviour here is what makes that requirement discoverable
/// rather than folklore.
#[test]
fn writing_over_a_wider_file_keeps_that_file_s_mode() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("was-public.json");
    std::fs::write(&path, "old").expect("seed");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("widen");

    termherd_pty::write_private(&path, "new").expect("write");

    let mode = std::fs::metadata(&path)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o777,
        0o644,
        "an existing file keeps its mode, got {:o}",
        mode & 0o777
    );
}
