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
//! codec seam), `walk` (the two-pass walk into records), `entry` (which
//! entries the walk enters and reads, and how much), and `repo` (the
//! repo-root helper). Dependency direction: `lib → walk → derive → cache`,
//! `walk` and `derive` both on the `entry` leaf, with `watch`/`repo`
//! independent.
//!
//! Rescans are incremental: the walk (`read_dir` + one stat per file)
//! always runs in full — it is what detects adds and removes — but a session
//! file is only re-read and re-digested when its mtime or size changed since
//! the previous scan. A live Claude session appending to one transcript costs
//! one file read per rescan, not one per session.
//!
//! The root is the caller's to resolve (`app` reads it from its `ClaudeDir`,
//! which honours `CLAUDE_CONFIG_DIR`): a second resolution here would drift.

mod antigravity;
mod cache;
mod derive;
mod entry;
mod open_file;
mod paths;
mod repo;
mod session_file;
mod walk;
mod watch;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use termherd_core::SessionRecord;
use termherd_core::ports::{ProjectScanner, ScanError};
use tracing::{debug, warn};

use antigravity::scan_antigravity;
use cache::ScanCache;
use walk::scan_root;

pub use paths::FsPathResolver;
pub use repo::{normalize_repo_path, repo_root, sidebar_key};
pub use session_file::{MAX_SESSION_FILE_BYTES, read_session_file};
pub use watch::{WatchHandle, watch_changes, watch_file};

#[derive(Default)]
pub(crate) struct ScannerCaches {
    pub(crate) claude: ScanCache,
    pub(crate) antigravity: ScanCache,
}

/// Scanner over a Claude projects root (normally `~/.claude/projects`),
/// optionally paired with an Antigravity data dir (`~/.gemini/antigravity-cli`).
pub struct FsScanner {
    root: PathBuf,
    antigravity_dir: Option<PathBuf>,
    /// Digest/cwd results of the previous scan, keyed by file signature.
    /// Interior mutability because [`ProjectScanner::scan`] takes
    /// `&self`; the shell serialises scans, so the lock is uncontended.
    pub(crate) caches: Mutex<ScannerCaches>,
}

impl FsScanner {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            antigravity_dir: None,
            caches: Mutex::new(ScannerCaches::default()),
        }
    }

    /// Also scan `dir` (normally `~/.gemini/antigravity-cli`) for Antigravity
    /// sessions alongside Claude's.
    #[must_use]
    pub fn with_antigravity_dir(mut self, dir: PathBuf) -> Self {
        self.antigravity_dir = Some(dir);
        self
    }

    /// The projects root this scanner walks.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl ProjectScanner for FsScanner {
    fn scan(&self) -> Result<Vec<SessionRecord>, ScanError> {
        let mut caches = self
            .caches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut outcome = scan_root(&self.root, &mut caches.claude)?;

        if let Some(agy_dir) = &self.antigravity_dir {
            let mut next_agy = ScanCache::default();
            let mut agy_records = scan_antigravity(agy_dir, &caches.antigravity, &mut next_agy);
            caches.antigravity = next_agy;
            outcome.records.append(&mut agy_records);
        }

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
