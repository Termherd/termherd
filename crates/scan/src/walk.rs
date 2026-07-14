//! Walking a projects root into session records. Runs the two-pass port of
//! upstream's `session-cache.readFolderFromFilesystem`: pass 1 derives (or
//! reuses) each folder's cwd, pass 2 digests its direct session files,
//! re-reading only the ones whose signature changed. Depends on [`crate::cache`]
//! and [`crate::derive`]; an antigravity walk would be a sibling source reusing
//! the same cache machinery.

use std::fs;
use std::path::{Path, PathBuf};

use termherd_claude::digest::digest_session;
use termherd_core::SessionRecord;
use termherd_core::ports::ScanError;

use crate::cache::{CachedCwd, CachedDigest, ScanCache, file_sig};
use crate::derive::{cached_cwd, derive_cwd, jsonl_files, resolve_worktree};

/// Result of walking a projects root: the derived session records, plus the
/// folders whose project path could not be derived (kept so the caller —
/// not this pure walk — decides how loudly to report them).
pub(crate) struct ScanOutcome {
    pub(crate) records: Vec<SessionRecord>,
    pub(crate) skipped: Vec<PathBuf>,
}

/// Walk a projects root, partitioning its folders into derived session
/// records and the folders that had no derivable cwd. Pure of logging policy
/// so the skip set is unit-testable. `cache` carries the previous scan's
/// digests and is replaced by this scan's generation.
pub(crate) fn scan_root(root: &Path, cache: &mut ScanCache) -> Result<ScanOutcome, ScanError> {
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
/// scan learns lands in `next`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FsScanner;
    use std::fs;
    use termherd_core::ports::ProjectScanner;

    fn write_session(dir: &Path, name: &str, cwd: &str, prompt: &str) {
        let line = format!("{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"message\":\"{prompt}\"}}\n");
        fs::write(dir.join(name), line).unwrap();
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
        // not re-read.
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
}
