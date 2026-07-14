//! termherd-scan — filesystem discovery adapter (`F-session-browser`, M1).
//!
//! Walks `~/.claude/projects`, derives each folder's real project path and
//! digests every session JSONL, using the pure `termherd-claude` codec.
//! Implements [`termherd_core::ports::ProjectScanner`].
//!
//! The walking order is a faithful port of upstream's
//! `deriveProjectPath` + `session-cache.readFolderFromFilesystem`
//! (`doctly/switchboard`): the project path comes from the first `cwd`
//! found in direct `*.jsonl` files, then in session subdirectories and
//! their `subagents/`; listed sessions are the *direct* `*.jsonl` files
//! only. A folder whose path cannot be derived is dropped, like upstream —
//! but logged, not silent (Q5).
//!
//! The concerns live in submodules under the crate root: `watch` (the
//! debounced fs watch behind FR2, an independent leaf), `cache` (the
//! incremental-scan signatures and reuse), `derive` (cwd derivation, the
//! codec seam), `walk` (the two-pass walk into records), and `repo` (the
//! repo-root helper). Dependency direction: `lib → walk → derive → cache`,
//! with `watch`/`repo` independent.
//!
//! Rescans are incremental: the walk (`read_dir` + one stat per file)
//! always runs in full — it is what detects adds and removes — but a session
//! file is only re-read and re-digested when its mtime or size changed since
//! the previous scan. A live Claude session appending to one transcript costs
//! one file read per rescan, not one per session.
//!
//! [`FsScanner::claude_default`] re-implements the `USERPROFILE`/`HOME`
//! precedence that `app::paths::home_dir` centralises. That duplication is
//! inherent to the hexagonal split — `scan` must not depend on `app` — so it
//! stays, deliberately, rather than being deduplicated across the wall.

mod cache;
mod derive;
mod repo;
mod walk;
mod watch;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use termherd_core::SessionRecord;
use termherd_core::ports::{ProjectScanner, ScanError};
use tracing::{debug, warn};

use cache::ScanCache;
use walk::scan_root;

pub use repo::repo_root;
pub use watch::{WatchHandle, watch_changes};

/// Scanner over a projects root (normally `~/.claude/projects`).
pub struct FsScanner {
    root: PathBuf,
    /// Digest/cwd results of the previous scan, keyed by file signature.
    /// Interior mutability because [`ProjectScanner::scan`] takes
    /// `&self`; the shell serialises scans, so the lock is uncontended.
    pub(crate) cache: Mutex<ScanCache>,
}

impl FsScanner {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            cache: Mutex::new(ScanCache::default()),
        }
    }

    /// Scanner over the default Claude CLI location, `~/.claude/projects`.
    /// `None` when no home directory can be determined.
    #[must_use]
    pub fn claude_default() -> Option<Self> {
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
        Some(Self::new(
            PathBuf::from(home).join(".claude").join("projects"),
        ))
    }

    /// The projects root this scanner walks.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl ProjectScanner for FsScanner {
    fn scan(&self) -> Result<Vec<SessionRecord>, ScanError> {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let outcome = scan_root(&self.root, &mut cache)?;

        // Q5: underivable folders are dropped like upstream, but never
        // silently. The detail is per-folder noise (dozens of `.worktrees`
        // checkouts skip cleanly on a busy machine), so it lives at `debug`;
        // default verbosity gets a single summary line, not a flood.
        for folder in &outcome.skipped {
            debug!(folder = %folder.display(), "no cwd derivable; folder skipped");
        }
        if !outcome.skipped.is_empty() {
            warn!(
                skipped = outcome.skipped.len(),
                "folders skipped (no cwd derivable); \
                 set RUST_LOG=termherd_scan=debug for the list"
            );
        }
        debug!(sessions = outcome.records.len(), root = %self.root.display(), "scan complete");
        Ok(outcome.records)
    }
}
