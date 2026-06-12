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
//! Still to come in M1: `notify` watching for live updates (FR2) and
//! `rayon` parallel parsing for large trees.

use std::fs;
use std::path::{Path, PathBuf};

use termherd_claude::derive::{collapse_worktree, extract_cwd};
use termherd_claude::digest::digest_session;
use termherd_core::SessionRecord;
use termherd_core::ports::{ProjectScanner, ScanError};
use tracing::{debug, warn};

/// Scanner over a projects root (normally `~/.claude/projects`).
pub struct FsScanner {
    root: PathBuf,
}

impl FsScanner {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
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
}

impl ProjectScanner for FsScanner {
    fn scan(&self) -> Result<Vec<SessionRecord>, ScanError> {
        let folders = fs::read_dir(&self.root)
            .map_err(|e| ScanError::Unreadable(format!("{}: {e}", self.root.display())))?;

        let mut records = Vec::new();
        for folder in folders.flatten() {
            let dir = folder.path();
            if !dir.is_dir() {
                continue;
            }
            match scan_folder(&dir) {
                Some(mut found) => records.append(&mut found),
                None => {
                    // Upstream drops underivable folders silently; we keep
                    // the behaviour but leave a trace.
                    warn!(folder = %dir.display(), "no cwd derivable; folder skipped");
                }
            }
        }
        debug!(sessions = records.len(), root = %self.root.display(), "scan complete");
        Ok(records)
    }
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
}
