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

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

use termherd_claude::derive::{collapse_worktree, extract_cwd};
use termherd_claude::digest::digest_session;
use termherd_core::SessionRecord;
use termherd_core::ports::{ProjectScanner, ScanError};
use tracing::{debug, warn};

/// Scanner over a projects root (normally `~/.claude/projects`).
pub struct FsScanner {
    root: PathBuf,
    /// The previous scan's `(records, skipped)` counts, packed into one word
    /// (`records << 32 | skipped`). Lets a steady-state rescan whose outcome is
    /// unchanged drop its skip summary to `debug!` instead of repeating the
    /// `warn!` every few seconds while a live session appends JSONL (#13).
    /// `u64::MAX` until the first scan, a value no real outcome can take.
    last_outcome: AtomicU64,
}

impl FsScanner {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            last_outcome: AtomicU64::new(u64::MAX),
        }
    }

    /// Whether this scan's `(records, skipped)` counts differ from the previous
    /// one, recording them for next time. The common live-session case — JSONL
    /// appended to existing files, so both counts hold steady — returns `false`,
    /// which is what demotes the repeated skip summary to `debug!` (#13). A new
    /// or vanished session moves a count and returns `true`, logging loudly.
    fn outcome_changed(&self, records: usize, skipped: usize) -> bool {
        let summary = ((records as u64) << 32) | (skipped as u64 & 0xFFFF_FFFF);
        self.last_outcome.swap(summary, Ordering::Relaxed) != summary
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

impl ProjectScanner for FsScanner {
    fn scan(&self) -> Result<Vec<SessionRecord>, ScanError> {
        let outcome = scan_root(&self.root)?;

        // The fs watcher fires a rescan on every debounced burst, and a live
        // session appends JSONL every few seconds — so without dedup this
        // summary drips endlessly. Log loudly only when the outcome actually
        // changed since the last scan; an unchanged steady state stays at
        // `debug` (#13).
        let changed = self.outcome_changed(outcome.records.len(), outcome.skipped.len());

        // Q5: underivable folders are dropped like upstream, but never
        // silently. The detail is per-folder noise (dozens of `.worktrees`
        // checkouts skip cleanly on a busy machine), so it lives at `debug`;
        // default verbosity gets a single summary line, not a flood.
        for folder in &outcome.skipped {
            debug!(folder = %folder.display(), "no cwd derivable; folder skipped");
        }
        if !outcome.skipped.is_empty() {
            if changed {
                warn!(
                    skipped = outcome.skipped.len(),
                    "folders skipped (no cwd derivable); \
                     set RUST_LOG=termherd_scan=debug for the list"
                );
            } else {
                debug!(
                    skipped = outcome.skipped.len(),
                    "folders skipped (unchanged since last scan)"
                );
            }
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
/// so the skip set is unit-testable.
fn scan_root(root: &Path) -> Result<ScanOutcome, ScanError> {
    let folders = fs::read_dir(root)
        .map_err(|e| ScanError::Unreadable(format!("{}: {e}", root.display())))?;

    let mut records = Vec::new();
    let mut skipped = Vec::new();
    for folder in folders.flatten() {
        let dir = folder.path();
        if !dir.is_dir() {
            continue;
        }
        match scan_folder(&dir) {
            Some(mut found) => records.append(&mut found),
            None => skipped.push(dir),
        }
    }
    Ok(ScanOutcome { records, skipped })
}

/// One project folder → its session records, or `None` when no project
/// path can be derived.
fn scan_folder(dir: &Path) -> Option<Vec<SessionRecord>> {
    let direct_jsonls = jsonl_files(dir);

    // Pass 1 — derive the folder's project path (upstream order: direct
    // files first, then session subdirectories, then their subagents/).
    let cwd = direct_jsonls
        .iter()
        .find_map(|p| extract_cwd(&fs::read_to_string(p).ok()?))
        .or_else(|| subdir_cwd(dir))?;
    let project_path = resolve_worktree(&cwd);

    // Pass 2 — digest the direct session files.
    let mut records = Vec::new();
    for path in direct_jsonls {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Some(digest) = digest_session(&content) else {
            continue;
        };
        let Some(session_id) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        let modified = fs::metadata(&path).and_then(|m| m.modified()).ok();
        records.push(SessionRecord {
            session_id,
            project_path: project_path.clone(),
            digest,
            modified,
        });
    }
    Some(records)
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
/// direct `*.jsonl`, or the first file under `subagents/`.
fn subdir_cwd(dir: &Path) -> Option<String> {
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
                return Some(cwd);
            }
        }
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

        let outcome = scan_root(tmp.path()).unwrap();
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
    fn outcome_change_detection_drives_the_skip_summary_log_level() {
        // #13: the first scan is always a change (nothing logged before); a
        // rescan with the same counts — the steady-state append case — is not,
        // so its skip summary drops to debug; a count that moves logs again.
        let scanner = FsScanner::new(PathBuf::from("/unused"));
        assert!(
            scanner.outcome_changed(213, 44),
            "first scan always changes"
        );
        assert!(
            !scanner.outcome_changed(213, 44),
            "identical counts must not re-log"
        );
        assert!(
            scanner.outcome_changed(214, 44),
            "a new session moves the record count"
        );
        assert!(
            scanner.outcome_changed(214, 43),
            "a vanished skip moves the skipped count"
        );
        assert!(
            !scanner.outcome_changed(214, 43),
            "settling back to a steady state stops logging again"
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
