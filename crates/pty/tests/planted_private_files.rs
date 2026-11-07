//! What another local user could plant first at the predictable name of a
//! session's private file, in the shared `/tmp` Linux uses: a world-readable
//! file or a symlink. Permissions and symlinks are Unix notions, so the whole
//! file is.
#![cfg(unix)]

use std::num::NonZeroU64;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use termherd_core::McpConfig;
use termherd_core::workspace::SessionId;
use termherd_pty::{PrivateFiles, write_mcp_config, write_title_settings};

fn session(id: u64) -> Option<SessionId> {
    NonZeroU64::new(id).map(SessionId)
}

/// A root for private files that no other process shares.
fn scratch() -> std::io::Result<(tempfile::TempDir, PrivateFiles)> {
    let tmp = tempfile::tempdir()?;
    let private = PrivateFiles::new(tmp.path());
    Ok((tmp, private))
}

fn mcp_config() -> McpConfig {
    McpConfig {
        url: "http://127.0.0.1:1/mcp".to_owned(),
        token: "feedface".to_owned(),
    }
}

#[test]
fn a_file_planted_world_readable_does_not_receive_the_token_as_is() {
    let (_tmp, private) = scratch().expect("scratch");
    let session = session(81).expect("nonzero");
    let path = private.mcp_config(session);
    std::fs::write(&path, "").expect("plant the file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))
        .expect("open it to everyone");

    let written =
        write_mcp_config(&private, session, &mcp_config()).expect("the config is written");
    let mode = std::fs::metadata(&written)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "got {mode:o}");
}

#[test]
fn a_symlink_planted_at_either_path_is_never_written_through() {
    type Writer = fn(&PrivateFiles, SessionId) -> Option<PathBuf>;
    type Named = fn(&PrivateFiles, SessionId) -> PathBuf;
    let writers: [(&str, u64, Named, Writer); 2] = [
        ("mcp config", 82, PrivateFiles::mcp_config, |p, s| {
            write_mcp_config(p, s, &mcp_config())
        }),
        (
            "settings overlay",
            83,
            PrivateFiles::settings,
            write_title_settings,
        ),
    ];
    for (name, id, path_of, write) in writers {
        let (_tmp, private) = scratch().expect("scratch");
        let session = session(id).expect("nonzero");
        let victim_dir = tempfile::tempdir().expect("the victim's dir");
        let victim = victim_dir.path().join("victim");
        std::fs::write(&victim, "victim").expect("the victim's file");
        let path = path_of(&private, session);
        std::os::unix::fs::symlink(&victim, &path).expect("plant the symlink");

        let _ = write(&private, session);
        assert_eq!(
            std::fs::read_to_string(&victim).expect("victim readable"),
            "victim",
            "the {name} write followed the planted symlink"
        );
        let kind = std::fs::symlink_metadata(&path).map(|meta| meta.file_type());
        assert!(
            !kind.is_ok_and(|kind| kind.is_symlink()),
            "the {name} path is still the planted symlink"
        );
    }
}
