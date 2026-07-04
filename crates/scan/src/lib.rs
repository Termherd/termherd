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
//! [`watch_changes`] provides the debounced fs watch behind live sidebar
//! updates (FR2). Still to come: `rayon` parallel parsing for large trees.
//!
//! Rescans are incremental (#133): the walk (`read_dir` + one stat per file)
//! always runs in full — it is what detects adds and removes — but a session
//! file is only re-read and re-digested when its mtime or size changed since
//! the previous scan. A live Claude session appending to one transcript costs
//! one file read per rescan, not one per session.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use notify::{RecursiveMode, Watcher};

use termherd_claude::derive::{collapse_worktree, extract_cwd};
use termherd_claude::digest::{SessionDigest, digest_session};
use termherd_core::SessionRecord;
use termherd_core::ports::{ProjectScanner, ScanError};
use tracing::{debug, warn};

/// Scanner over a projects root (normally `~/.claude/projects`).
pub struct FsScanner {
    root: PathBuf,
    /// Digest/cwd results of the previous scan, keyed by file signature
    /// (#133). Interior mutability because [`ProjectScanner::scan`] takes
    /// `&self`; the shell serialises scans, so the lock is uncontended.
    cache: Mutex<ScanCache>,
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

/// Keeps the fs watcher and its coalescing thread alive; dropping it stops
/// both.
pub struct WatchHandle {
    _watcher: notify::RecommendedWatcher,
}

/// Watch `root` recursively and invoke `on_change` once per debounced
/// burst of filesystem events (the CLI appends JSONL lines continuously;
/// without coalescing every keystroke of a session would trigger a
/// rescan). The callback runs on a background thread.
pub fn watch_changes(
    root: PathBuf,
    debounce: Duration,
    mut on_change: impl FnMut() + Send + 'static,
) -> Result<WatchHandle, ScanError> {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok() {
            let _ = tx.send(());
        }
    })
    .map_err(|e| ScanError::Unreadable(format!("watcher: {e}")))?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| ScanError::Unreadable(format!("{}: {e}", root.display())))?;

    std::thread::Builder::new()
        .name("termherd-fs-watch".into())
        .spawn(move || {
            // One blocking recv starts a burst; keep draining until the
            // tree has been quiet for `debounce`, then fire once.
            while rx.recv().is_ok() {
                while rx.recv_timeout(debounce).is_ok() {}
                on_change();
            }
            debug!("fs watch channel closed; coalescing thread exiting");
        })
        .map_err(|e| ScanError::Unreadable(format!("watch thread: {e}")))?;

    Ok(WatchHandle { _watcher: watcher })
}

/// A file's cache-invalidation signature (#133): mtime + size. Either
/// changing marks the cached derivation stale; requiring both to match
/// mitigates coarse mtime granularity on some filesystems.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileSig {
    mtime: SystemTime,
    size: u64,
}

/// The signature `path` currently carries, or `None` when it cannot be
/// stat'ed — then the file is treated as always-dirty, never cached.
fn file_sig(path: &Path) -> Option<FileSig> {
    let meta = fs::metadata(path).ok()?;
    Some(FileSig {
        mtime: meta.modified().ok()?,
        size: meta.len(),
    })
}

/// One prior digest: reused while the file's signature is unchanged. `None`
/// digests (an unparsable transcript) are cached too, so a permanently
/// invalid file is not re-read on every scan.
struct CachedDigest {
    sig: FileSig,
    digest: Option<SessionDigest>,
}

/// One prior cwd derivation for a folder: reused while the transcript it
/// came from is unchanged.
struct CachedCwd {
    source: PathBuf,
    sig: FileSig,
    cwd: String,
}

/// What the previous scan learned (#133). Each scan builds a fresh
/// generation and replaces the old wholesale, so entries for files and
/// folders that vanished are pruned by never being carried over.
#[derive(Default)]
struct ScanCache {
    digests: HashMap<PathBuf, CachedDigest>,
    cwds: HashMap<PathBuf, CachedCwd>,
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

/// Result of walking a projects root: the derived session records, plus the
/// folders whose project path could not be derived (kept so the caller —
/// not this pure walk — decides how loudly to report them).
struct ScanOutcome {
    records: Vec<SessionRecord>,
    skipped: Vec<PathBuf>,
}

/// Walk a projects root, partitioning its folders into derived session
/// records and the folders that had no derivable cwd. Pure of logging policy
/// so the skip set is unit-testable. `cache` carries the previous scan's
/// digests (#133) and is replaced by this scan's generation.
fn scan_root(root: &Path, cache: &mut ScanCache) -> Result<ScanOutcome, ScanError> {
    let folders = fs::read_dir(root)
        .map_err(|e| ScanError::Unreadable(format!("{}: {e}", root.display())))?;

    let mut records = Vec::new();
    let mut skipped = Vec::new();
    let mut next = ScanCache::default();
    for folder in folders.flatten() {
        let dir = folder.path();
        if !dir.is_dir() {
            continue;
        }
        match scan_folder(&dir, cache, &mut next) {
            Some(mut found) => records.append(&mut found),
            None => skipped.push(dir),
        }
    }
    *cache = next;
    Ok(ScanOutcome { records, skipped })
}

/// One project folder → its session records, or `None` when no project
/// path can be derived. Unchanged files reuse `old`'s digests; whatever this
/// scan learns lands in `next` (#133).
fn scan_folder(dir: &Path, old: &ScanCache, next: &mut ScanCache) -> Option<Vec<SessionRecord>> {
    let direct_jsonls = jsonl_files(dir);

    // Pass 1 — the folder's cwd: reused while the transcript it came from is
    // unchanged, else re-derived (upstream order: direct files first, then
    // session subdirectories, then their subagents/).
    let (source, cwd) = cached_cwd(dir, old).or_else(|| derive_cwd(dir, &direct_jsonls))?;
    if let Some(sig) = file_sig(&source) {
        next.cwds.insert(
            dir.to_owned(),
            CachedCwd {
                source,
                sig,
                cwd: cwd.clone(),
            },
        );
    }
    // The worktree collapse re-runs every scan: it depends on the parent
    // existing on disk *now*, not on the transcript the cwd came from.
    let project_path = resolve_worktree(&cwd);

    // Pass 2 — digest the direct session files, re-reading only the ones
    // whose signature changed since the previous scan.
    let mut records = Vec::new();
    for path in direct_jsonls {
        let Some(session_id) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        let sig = file_sig(&path);
        let digest = match (sig, old.digests.get(&path)) {
            (Some(sig), Some(hit)) if hit.sig == sig => hit.digest.clone(),
            _ => fs::read_to_string(&path)
                .ok()
                .and_then(|c| digest_session(&c)),
        };
        if let Some(sig) = sig {
            next.digests.insert(
                path.clone(),
                CachedDigest {
                    sig,
                    digest: digest.clone(),
                },
            );
        }
        let Some(digest) = digest else {
            continue;
        };
        records.push(SessionRecord {
            session_id,
            project_path: project_path.clone(),
            digest,
            modified: sig.map(|s| s.mtime),
        });
    }
    Some(records)
}

/// The folder's previously derived cwd, if the transcript it came from still
/// carries the same signature (#133).
fn cached_cwd(dir: &Path, cache: &ScanCache) -> Option<(PathBuf, String)> {
    let hit = cache.cwds.get(dir)?;
    (file_sig(&hit.source)? == hit.sig).then(|| (hit.source.clone(), hit.cwd.clone()))
}

/// Derive the folder's cwd from scratch, returning the transcript it came
/// from so the derivation can be cached against that file's signature.
fn derive_cwd(dir: &Path, direct_jsonls: &[PathBuf]) -> Option<(PathBuf, String)> {
    direct_jsonls
        .iter()
        .find_map(|p| Some((p.clone(), extract_cwd(&fs::read_to_string(p).ok()?)?)))
        .or_else(|| subdir_cwd(dir))
}

/// Direct `*.jsonl` files of a folder, in directory order like upstream.
fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "jsonl"))
        .collect()
}

/// Fallback cwd source: session subdirectories (UUID folders) — their
/// direct `*.jsonl`, or the first file under `subagents/`. Returns the file
/// the cwd came from alongside it, for the derivation cache (#133).
fn subdir_cwd(dir: &Path) -> Option<(PathBuf, String)> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let sub = entry.path();
        if !sub.is_dir() {
            continue;
        }
        let mut candidates = jsonl_files(&sub);
        candidates.extend(jsonl_files(&sub.join("subagents")).into_iter().take(1));
        for candidate in candidates {
            if let Ok(content) = fs::read_to_string(&candidate)
                && let Some(cwd) = extract_cwd(&content)
            {
                return Some((candidate, cwd));
            }
        }
    }
    None
}

/// The repository root for `start`: the nearest ancestor (including `start`
/// itself) that holds a `.git` entry, or `None` if none does. The entry may be
/// a directory (a normal clone) or a file (a submodule or linked worktree
/// `.git` pointer), so both count.
///
/// Used by the "new Claude session in the same repo" shortcut (#77): a session
/// may be running in a subdirectory, so the launch walks up to the repo root
/// rather than reusing the literal cwd.
#[must_use]
pub fn repo_root(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(current) = dir {
        if current.join(".git").exists() {
            return Some(current.to_owned());
        }
        dir = current.parent();
    }
    None
}

/// Collapse a worktree checkout onto its main project — only when the
/// candidate parent actually exists, like upstream's `fs.existsSync`.
fn resolve_worktree(cwd: &str) -> String {
    match collapse_worktree(cwd) {
        Some(parent) if Path::new(parent).exists() => parent.to_owned(),
        _ => cwd.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_session(dir: &Path, name: &str, cwd: &str, prompt: &str) {
        let line = format!("{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"message\":\"{prompt}\"}}\n");
        fs::write(dir.join(name), line).unwrap();
    }

    #[test]
    fn repo_root_finds_the_nearest_dot_git_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let nested = repo.join("crates").join("app");
        fs::create_dir_all(&nested).unwrap();
        // A normal clone: `.git` is a directory at the repo root.
        fs::create_dir(repo.join(".git")).unwrap();

        // From a deep subdirectory, the walk climbs to the repo root.
        assert_eq!(repo_root(&nested).as_deref(), Some(repo.as_path()));
        // From the root itself, it returns the root.
        assert_eq!(repo_root(&repo).as_deref(), Some(repo.as_path()));
    }

    #[test]
    fn repo_root_accepts_a_dot_git_file_and_returns_none_outside_a_repo() {
        let tmp = tempfile::tempdir().unwrap();
        // A linked worktree / submodule: `.git` is a file pointer, not a dir.
        let worktree = tmp.path().join("wt");
        fs::create_dir(&worktree).unwrap();
        fs::write(worktree.join(".git"), "gitdir: /somewhere\n").unwrap();
        assert_eq!(repo_root(&worktree).as_deref(), Some(worktree.as_path()));

        // A directory with no `.git` anywhere above it has no repo root.
        let bare = tmp.path().join("bare");
        fs::create_dir(&bare).unwrap();
        assert_eq!(repo_root(&bare), None);
    }

    #[test]
    fn scans_direct_sessions_with_derived_path() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("C--proj");
        fs::create_dir(&folder).unwrap();
        write_session(&folder, "abc.jsonl", "/real/proj", "hello");
        write_session(&folder, "def.jsonl", "/real/proj", "world");

        let records = FsScanner::new(tmp.path().to_owned()).scan().unwrap();
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|r| r.project_path == "/real/proj"));
        assert!(records.iter().any(|r| r.session_id == "abc"));
    }

    #[test]
    fn falls_back_to_subagent_cwd_for_the_folder_path() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("C--proj");
        let subagents = folder.join("some-uuid").join("subagents");
        fs::create_dir_all(&subagents).unwrap();
        // The direct session has no cwd; only the subagent transcript does.
        fs::write(
            folder.join("abc.jsonl"),
            "{\"type\":\"user\",\"message\":\"no cwd here\"}\n",
        )
        .unwrap();
        write_session(&subagents, "agent.jsonl", "/from/subagent", "x");

        let records = FsScanner::new(tmp.path().to_owned()).scan().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].project_path, "/from/subagent");
    }

    #[test]
    fn underivable_folders_are_dropped_like_upstream() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("C--mystery");
        fs::create_dir(&folder).unwrap();
        fs::write(
            folder.join("abc.jsonl"),
            "{\"type\":\"user\",\"message\":\"hi\"}\n",
        )
        .unwrap();

        let records = FsScanner::new(tmp.path().to_owned()).scan().unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn underivable_folders_are_reported_in_the_outcome_not_dropped_silently() {
        // The skip is surfaced to the caller (Q5: not silent) as data, so the
        // logging policy — one summary line, not one per folder — can be
        // applied without re-walking. Two underivable folders, one good one.
        let tmp = tempfile::tempdir().unwrap();

        let good = tmp.path().join("C--proj");
        fs::create_dir(&good).unwrap();
        write_session(&good, "abc.jsonl", "/real/proj", "hello");

        for name in ["C--mystery", "C--ghost"] {
            let folder = tmp.path().join(name);
            fs::create_dir(&folder).unwrap();
            fs::write(
                folder.join("abc.jsonl"),
                "{\"type\":\"user\",\"message\":\"no cwd\"}\n",
            )
            .unwrap();
        }

        let outcome = scan_root(tmp.path(), &mut ScanCache::default()).unwrap();
        assert_eq!(outcome.records.len(), 1);
        assert_eq!(outcome.skipped.len(), 2);
        assert!(
            outcome
                .skipped
                .iter()
                .all(|p| p.file_name().is_some_and(|n| n != "C--proj"))
        );
    }

    #[test]
    fn worktree_paths_collapse_when_the_parent_exists() {
        let tmp = tempfile::tempdir().unwrap();
        // A "main checkout" that exists on disk.
        let main = tmp.path().join("proj");
        fs::create_dir(&main).unwrap();
        let cwd = format!(
            "{}/.worktrees/feat",
            main.display().to_string().replace('\\', "/")
        );

        let folder = tmp.path().join("C--proj-worktrees-feat");
        fs::create_dir(&folder).unwrap();
        write_session(&folder, "abc.jsonl", &cwd, "x");

        let records = FsScanner::new(tmp.path().to_owned()).scan().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].project_path,
            main.display().to_string().replace('\\', "/")
        );

        // Same shape, but the parent does not exist → keep the worktree cwd.
        let orphan_folder = tmp.path().join("C--ghost");
        fs::create_dir(&orphan_folder).unwrap();
        write_session(&orphan_folder, "xyz.jsonl", "/ghost/.worktrees/feat", "x");
        let records = FsScanner::new(tmp.path().to_owned()).scan().unwrap();
        let ghost = records.iter().find(|r| r.session_id == "xyz").unwrap();
        assert_eq!(ghost.project_path, "/ghost/.worktrees/feat");
    }

    /// Overwrite `path` with `content` but restore its original mtime, so the
    /// cache signature (mtime+size) is unchanged if the length matches. This
    /// is how the tests *prove* a cache hit: a re-read would see the new
    /// content, a hit keeps serving the old digest.
    fn rewrite_preserving_sig(path: &Path, content: &str) {
        let mtime = fs::metadata(path).unwrap().modified().unwrap();
        fs::write(path, content).unwrap();
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_times(fs::FileTimes::new().set_modified(mtime))
            .unwrap();
    }

    #[test]
    fn unchanged_files_are_served_from_the_digest_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("C--proj");
        fs::create_dir(&folder).unwrap();
        write_session(&folder, "abc.jsonl", "/real/proj", "hello");

        let scanner = FsScanner::new(tmp.path().to_owned());
        let first = scanner.scan().unwrap();
        assert_eq!(first[0].digest.summary, "hello");

        // Same signature (length preserved, mtime restored) but different
        // content: a cache hit keeps the old digest — proof the file was
        // not re-read (#133).
        let line = "{\"type\":\"user\",\"cwd\":\"/real/proj\",\"message\":\"jello\"}\n";
        rewrite_preserving_sig(&folder.join("abc.jsonl"), line);
        let second = scanner.scan().unwrap();
        assert_eq!(second[0].digest.summary, "hello", "cache hit expected");
    }

    #[test]
    fn a_changed_signature_invalidates_the_cached_digest() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("C--proj");
        fs::create_dir(&folder).unwrap();
        write_session(&folder, "abc.jsonl", "/real/proj", "hello");

        let scanner = FsScanner::new(tmp.path().to_owned());
        scanner.scan().unwrap();

        // A longer transcript (size changes → signature changes) re-digests.
        write_session(&folder, "abc.jsonl", "/real/proj", "hello and more");
        let second = scanner.scan().unwrap();
        assert_eq!(second[0].digest.summary, "hello and more");
    }

    #[test]
    fn removed_files_leave_no_stale_record_or_cache_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("C--proj");
        fs::create_dir(&folder).unwrap();
        write_session(&folder, "abc.jsonl", "/real/proj", "hello");
        write_session(&folder, "def.jsonl", "/real/proj", "world");

        let scanner = FsScanner::new(tmp.path().to_owned());
        assert_eq!(scanner.scan().unwrap().len(), 2);

        fs::remove_file(folder.join("def.jsonl")).unwrap();
        let after = scanner.scan().unwrap();
        assert_eq!(after.len(), 1, "the removed session is gone");
        // The cache generation was rebuilt without the removed file.
        let cache = scanner
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(cache.digests.len(), 1);
    }

    #[test]
    fn the_cwd_derivation_is_cached_but_follows_its_source() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("C--proj");
        fs::create_dir(&folder).unwrap();
        write_session(&folder, "abc.jsonl", "/real/proj", "hello");

        let scanner = FsScanner::new(tmp.path().to_owned());
        assert_eq!(scanner.scan().unwrap()[0].project_path, "/real/proj");

        // The source transcript changes cwd (size differs → re-derived).
        write_session(&folder, "abc.jsonl", "/moved/projects", "hello");
        assert_eq!(scanner.scan().unwrap()[0].project_path, "/moved/projects");
    }

    #[test]
    fn missing_root_is_a_typed_error() {
        let scanner = FsScanner::new(PathBuf::from("/definitely/not/here"));
        assert!(scanner.scan().is_err());
    }

    #[test]
    fn watch_fires_once_per_debounced_burst() {
        let tmp = tempfile::tempdir().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let _handle = watch_changes(
            tmp.path().to_owned(),
            std::time::Duration::from_millis(200),
            move || {
                let _ = tx.send(());
            },
        )
        .unwrap();

        // A burst of writes…
        for i in 0..5 {
            fs::write(tmp.path().join(format!("f{i}.jsonl")), "x").unwrap();
        }
        // …yields at least one change signal (fs event latency varies by
        // platform, so allow a generous window but require coalescing to
        // have collapsed the burst into very few signals).
        assert!(
            rx.recv_timeout(std::time::Duration::from_secs(10)).is_ok(),
            "no change signal within 10s"
        );
        let extra =
            std::iter::from_fn(|| rx.recv_timeout(std::time::Duration::from_millis(600)).ok())
                .count();
        assert!(
            extra <= 2,
            "burst was not coalesced: {} extra signals",
            extra + 1
        );
    }
}
