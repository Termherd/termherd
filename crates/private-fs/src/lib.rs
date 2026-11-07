//! Owner-only files and directories: what termherd writes that nobody else on
//! the machine may read or plant. One home for the permission fork, so every
//! writer of a private file gets the same rules rather than its own copy.

use std::path::Path;

/// Create `path` as a directory only this user can reach (`0o700` on Unix), and
/// **fail if it already exists**.
///
/// Both halves are load-bearing. The default temp directory is per-user on
/// macOS but is the shared, world-writable `/tmp` on Linux, and what goes in
/// there may be *sourced by the user's login shell*. A directory another local
/// user pre-created under the name we are about to use would let them choose
/// what that shell executes. Refusing to adopt an existing directory leaves the
/// caller to fall back; the private mode keeps the files unreadable and
/// unwritable to anyone else afterwards.
#[cfg(unix)]
pub fn create_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    std::fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
pub fn create_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}

/// Create `path` afresh, owner-only (`0o600` on Unix), and hand back the open
/// file — for a writer that streams, as a screencast does.
///
/// Never writes through an entry already there: on Linux's shared `/tmp`, a
/// planted file keeps its own mode and a planted symlink redirects the write.
/// Our own leftover is removed first; an entry we cannot remove fails the
/// create.
pub fn create_file(path: &Path) -> std::io::Result<std::fs::File> {
    let _ = std::fs::remove_file(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    owner_only(&mut options);
    options.open(path)
}

/// Create `path` afresh, owner-only, holding `contents` (see [`create_file`]).
pub fn write_file(path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    use std::io::Write;

    create_file(path)?.write_all(contents.as_ref())
}

/// Make sure `path` is a directory only this user can reach, creating it (and
/// any missing parent) if need be.
///
/// Unlike [`create_dir`], an existing directory is adopted — for one that lives
/// under the user's own data dir and outlives the run — but its mode is
/// tightened to `0o700`, so a directory an older build left world-readable
/// stops exposing what it holds.
#[cfg(unix)]
pub fn ensure_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
pub fn ensure_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

/// Whether nobody but the owner can read, write or enter `path`. Always true
/// off Unix, where a file under the user's profile inherits its owner-only ACL
/// and there is no mode to check.
#[cfg(unix)]
pub fn is_private(path: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::PermissionsExt;

    Ok(std::fs::metadata(path)?.permissions().mode() & 0o077 == 0)
}

#[cfg(not(unix))]
pub fn is_private(path: &Path) -> std::io::Result<bool> {
    std::fs::metadata(path).map(|_| true)
}

#[cfg(unix)]
fn owner_only(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.mode(0o600);
}

#[cfg(not(unix))]
fn owner_only(_: &mut std::fs::OpenOptions) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_that_is_already_there_is_never_adopted() {
        // What goes in may be *sourced by the user's login shell*, and the
        // default temp directory is the shared `/tmp` on Linux. Adopting a
        // directory already in place would let whoever created it choose what
        // that shell runs.
        let tmp = tempfile::tempdir().expect("tempdir");
        let planted = tmp.path().join("planted");
        std::fs::create_dir_all(&planted).expect("plant the directory");

        assert!(
            create_dir(&planted).is_err(),
            "an existing directory must never be adopted"
        );
    }
}
