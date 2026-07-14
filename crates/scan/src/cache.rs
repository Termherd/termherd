//! Incremental-scan cache: a file's invalidation signature plus the reused
//! digest and cwd derivations. A scan builds a fresh [`ScanCache`] generation
//! and replaces the old one wholesale, so vanished files and folders are pruned
//! by never being carried over.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use termherd_claude::digest::SessionDigest;

/// A file's cache-invalidation signature: mtime + size. Either
/// changing marks the cached derivation stale; requiring both to match
/// mitigates coarse mtime granularity on some filesystems.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileSig {
    pub(crate) mtime: SystemTime,
    pub(crate) size: u64,
}

/// The signature `path` currently carries, or `None` when it cannot be
/// stat'ed — then the file is treated as always-dirty, never cached.
pub(crate) fn file_sig(path: &Path) -> Option<FileSig> {
    let meta = fs::metadata(path).ok()?;
    Some(FileSig {
        mtime: meta.modified().ok()?,
        size: meta.len(),
    })
}

/// One prior digest: reused while the file's signature is unchanged. `None`
/// digests (an unparsable transcript) are cached too, so a permanently
/// invalid file is not re-read on every scan.
pub(crate) struct CachedDigest {
    pub(crate) sig: FileSig,
    pub(crate) digest: Option<SessionDigest>,
}

/// One prior cwd derivation for a folder: reused while the transcript it
/// came from is unchanged.
pub(crate) struct CachedCwd {
    pub(crate) source: PathBuf,
    pub(crate) sig: FileSig,
    pub(crate) cwd: String,
}

/// What the previous scan learned. Each scan builds a fresh
/// generation and replaces the old wholesale, so entries for files and
/// folders that vanished are pruned by never being carried over.
#[derive(Default)]
pub(crate) struct ScanCache {
    pub(crate) digests: HashMap<PathBuf, CachedDigest>,
    pub(crate) cwds: HashMap<PathBuf, CachedCwd>,
}
