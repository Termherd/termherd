//! cwd derivation: a folder's real project path from its transcripts. The
//! order is a faithful port of upstream's `deriveProjectPath` — direct
//! `*.jsonl` files first, then session subdirectories and their `subagents/` —
//! plus the worktree collapse. Plugs into the pure `termherd_claude` codec, the
//! seam where an alternate transcript parser (e.g. antigravity) would attach.

use std::fs;
use std::path::{Path, PathBuf};

use termherd_claude::derive::{collapse_worktree, extract_cwd};

use crate::cache::{ScanCache, file_sig};

/// The folder's previously derived cwd, if the transcript it came from still
/// carries the same signature.
pub(crate) fn cached_cwd(dir: &Path, cache: &ScanCache) -> Option<(PathBuf, String)> {
    let hit = cache.cwds.get(dir)?;
    (file_sig(&hit.source)? == hit.sig).then(|| (hit.source.clone(), hit.cwd.clone()))
}

/// Derive the folder's cwd from scratch, returning the transcript it came
/// from so the derivation can be cached against that file's signature.
pub(crate) fn derive_cwd(dir: &Path, direct_jsonls: &[PathBuf]) -> Option<(PathBuf, String)> {
    direct_jsonls
        .iter()
        .find_map(|p| Some((p.clone(), extract_cwd(&fs::read_to_string(p).ok()?)?)))
        .or_else(|| subdir_cwd(dir))
}

/// Direct `*.jsonl` files of a folder, in directory order like upstream.
pub(crate) fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
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
/// the cwd came from alongside it, for the derivation cache.
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

/// Collapse a worktree checkout onto its main project — only when the
/// candidate parent actually exists, like upstream's `fs.existsSync`.
pub(crate) fn resolve_worktree(cwd: &str) -> String {
    match collapse_worktree(cwd) {
        Some(parent) if Path::new(parent).exists() => parent.to_owned(),
        _ => cwd.to_owned(),
    }
}
