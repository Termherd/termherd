//! Plans & memory discovery, reading, and editing (`F-plans-memory`). Lists the
//! plan files under `~/.claude/plans`, the global `~/.claude/CLAUDE.md`, and a
//! `CLAUDE.md` for each known project that has one; reads a file on demand; and
//! writes one back atomically, but only within the narrow write-scope enforced
//! by [`termherd_core::docscope`] (ADR 0001). A file adapter owned by the
//! shell, like [`crate::settings`].

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Which kind of document an entry is — drives its icon/grouping in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Plan,
    GlobalMemory,
    ProjectMemory,
}

/// One browsable document: where it lives and how to label it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocEntry {
    pub kind: DocKind,
    pub label: String,
    pub path: PathBuf,
}

/// Discover the global memory, the plan files, and a `CLAUDE.md` for each given
/// project path that has one. Order: global memory, plans (by name), then
/// project memories in the order given. Missing files are simply absent.
#[must_use]
pub fn discover(project_paths: &[String]) -> Vec<DocEntry> {
    let mut docs = Vec::new();
    if let Some(home) = claude_home() {
        let global = home.join("CLAUDE.md");
        if global.is_file() {
            docs.push(DocEntry {
                kind: DocKind::GlobalMemory,
                label: "CLAUDE.md (global)".to_owned(),
                path: global,
            });
        }
        if let Ok(entries) = std::fs::read_dir(home.join("plans")) {
            let mut plans: Vec<DocEntry> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
                .map(|p| DocEntry {
                    kind: DocKind::Plan,
                    label: plan_label(&p),
                    path: p,
                })
                .collect();
            plans.sort_by(|a, b| a.label.cmp(&b.label));
            docs.extend(plans);
        }
    }
    for path in project_paths {
        let candidate = Path::new(path).join("CLAUDE.md");
        if candidate.is_file() {
            docs.push(DocEntry {
                kind: DocKind::ProjectMemory,
                label: format!("CLAUDE.md · {}", last_component(path)),
                path: candidate,
            });
        }
    }
    docs
}

/// Read a document's text. Errors surface to the caller (shown in the viewer).
pub fn read(path: &Path) -> std::io::Result<String> {
    std::fs::read_to_string(path)
}

/// A document's last-modified time, captured when it is opened so a save can
/// detect a concurrent writer (a live Claude process editing the same file).
pub fn mtime(path: &Path) -> std::io::Result<SystemTime> {
    std::fs::metadata(path)?.modified()
}

/// Why an in-app save was refused or failed. `Clone` so it can ride an iced
/// `Message`; the underlying io error is rendered to a string.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum SaveError {
    /// The path is not in the writable document scope (`core::docscope`).
    #[error("not a writable document")]
    OutOfScope,
    /// The file changed on disk since it was opened — overwriting would lose
    /// the concurrent writer's changes.
    #[error("file changed on disk since it was opened")]
    Conflict,
    /// The write itself failed (permissions, read-only fs, …).
    #[error("write failed: {0}")]
    Io(String),
}

/// Whether the in-app editor may write `path`, resolving the user's `~/.claude`
/// itself. Drives whether the viewer offers a Save action.
#[must_use]
pub fn is_writable(path: &Path) -> bool {
    termherd_core::docscope::is_writable(path, &home_or_sentinel())
}

/// Save `contents` to `path` (the shell-facing entry point), resolving the
/// `~/.claude` boundary. `open_mtime` is the baseline captured when the file was
/// opened, or `None` if it could not be read (then the concurrency guard is
/// skipped — there is nothing to compare against). Returns the file's new mtime
/// so the caller can refresh its baseline.
pub fn save(
    path: &Path,
    contents: &str,
    open_mtime: Option<SystemTime>,
) -> Result<SystemTime, SaveError> {
    write(path, contents, open_mtime, &home_or_sentinel())
}

/// Write `contents` to `path`, but only if (a) the path is within the writable
/// document scope and (b) the file has not changed on disk since `open_mtime`.
/// The write is atomic — a temp file in the same directory, then a rename — so
/// a crash mid-write never truncates the original. Returns the file's mtime
/// after the write.
///
/// `claude_home` is the user's `~/.claude`; it is the boundary the scope
/// predicate is measured against. Split from [`save`] so the boundary is an
/// explicit argument the tests can drive without touching the environment.
fn write(
    path: &Path,
    contents: &str,
    open_mtime: Option<SystemTime>,
    claude_home: &Path,
) -> Result<SystemTime, SaveError> {
    use termherd_core::docscope::{SaveDecision, decide_save, is_writable};

    if !is_writable(path, claude_home) {
        return Err(SaveError::OutOfScope);
    }
    // With a baseline and a file still on disk, refuse if it changed under us.
    if let (Some(open), Ok(on_disk)) = (open_mtime, mtime(path))
        && decide_save(open, on_disk) == SaveDecision::Conflict
    {
        return Err(SaveError::Conflict);
    }
    atomic_write(path, contents).map_err(|e| SaveError::Io(e.to_string()))?;
    mtime(path).map_err(|e| SaveError::Io(e.to_string()))
}

/// Write `contents` to `path` via a temp file in the same directory followed by
/// a rename, so an interrupted write never truncates the original.
fn atomic_write(path: &Path, contents: &str) -> std::io::Result<()> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = directory.join(format!(".{file_name}.termherd.tmp"));
    std::fs::write(&temp, contents)?;
    std::fs::rename(&temp, path)
}

/// A plan's display label: its file stem (the slug Claude assigns).
fn plan_label(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("plan")
        .to_owned()
}

/// The last path component, for labelling a project's memory file.
fn last_component(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path)
}

/// `~/.claude` — home of plans and memory.
fn claude_home() -> Option<PathBuf> {
    Some(crate::paths::home_dir()?.join(".claude"))
}

/// `~/.claude` if it resolves, else a path that is the prefix of nothing — so a
/// project `CLAUDE.md` (outside any home) is still writable while no file can be
/// mistaken for one inside the protected tree.
fn home_or_sentinel() -> PathBuf {
    claude_home().unwrap_or_else(|| PathBuf::from("\0/no-claude-home"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_label_is_the_file_stem() {
        assert_eq!(
            plan_label(Path::new("/x/.claude/plans/atomic-purring-torvalds.md")),
            "atomic-purring-torvalds"
        );
    }

    #[test]
    fn last_component_handles_both_separators() {
        assert_eq!(last_component("/home/me/proj"), "proj");
        assert_eq!(last_component(r"C:\projets\termherd"), "termherd");
        assert_eq!(last_component("/trailing/slash/"), "slash");
    }

    // --- write slice -------------------------------------------------------

    /// Build a fake `~/.claude` with a plan and a project memory under `root`.
    fn scaffold(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let home = root.join(".claude");
        std::fs::create_dir_all(home.join("plans")).expect("mkdir plans");
        std::fs::create_dir_all(home.join("projects").join("-repo")).expect("mkdir projects");
        let plan = home.join("plans").join("my-plan.md");
        std::fs::write(&plan, "old plan").expect("seed plan");
        let session = home.join("projects").join("-repo").join("abc.jsonl");
        std::fs::write(&session, "{}").expect("seed session");
        (home, plan, session)
    }

    #[test]
    fn is_writable_wrapper_resolves_the_home_boundary() {
        // The env-resolving wrapper must agree with the scope predicate: a
        // project CLAUDE.md (outside ~/.claude) is writable, an arbitrary file
        // is not. This pins the wrapper + home resolution, not just the pure
        // core predicate.
        assert!(is_writable(Path::new("/tmp/some-repo/CLAUDE.md")));
        assert!(!is_writable(Path::new("/tmp/some-repo/notes.txt")));
    }

    #[test]
    fn writes_a_plan_atomically() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (home, plan, _) = scaffold(dir.path());
        let m = mtime(&plan).expect("mtime");

        write(&plan, "new plan body", Some(m), &home).expect("save ok");

        assert_eq!(
            std::fs::read_to_string(&plan).expect("read"),
            "new plan body"
        );
    }

    #[test]
    fn refuses_to_write_outside_scope() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (home, _, session) = scaffold(dir.path());
        let m = mtime(&session).expect("mtime");

        let err = write(&session, "tampered", Some(m), &home).expect_err("must reject");

        assert_eq!(err, SaveError::OutOfScope);
        // Session JSONL is untouched.
        assert_eq!(std::fs::read_to_string(&session).expect("read"), "{}");
    }

    /// Regression: a project `CLAUDE.md` that `discover` surfaces must be one
    /// the write-scope predicate also accepts — the browser never offers a doc
    /// the save path would then reject.
    #[test]
    fn discovered_project_memory_is_writable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let proj = dir.path().join("repo");
        std::fs::create_dir_all(&proj).expect("mkdir repo");
        std::fs::write(proj.join("CLAUDE.md"), "# project").expect("seed");

        let proj_str = proj.to_string_lossy().into_owned();
        let docs = discover(&[proj_str]);

        let entry = docs
            .iter()
            .find(|d| d.kind == DocKind::ProjectMemory)
            .expect("project memory discovered");
        assert!(termherd_core::docscope::is_writable(
            &entry.path,
            &dir.path().join(".claude"),
        ));
    }

    #[test]
    fn saves_without_a_baseline_when_mtime_was_unreadable() {
        // No open-time mtime ⇒ no concurrency baseline to compare against, so
        // the save proceeds rather than being wedged into a permanent conflict.
        let dir = tempfile::tempdir().expect("tempdir");
        let (home, plan, _) = scaffold(dir.path());

        write(&plan, "rescued", None, &home).expect("save ok");

        assert_eq!(std::fs::read_to_string(&plan).expect("read"), "rescued");
    }

    #[test]
    fn refuses_to_overwrite_a_file_changed_on_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (home, plan, _) = scaffold(dir.path());
        let stale = mtime(&plan).expect("mtime");

        // A concurrent writer touches the file after we captured `stale`.
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&plan, "claude wrote this").expect("concurrent write");

        let err = write(&plan, "our edit", Some(stale), &home).expect_err("must conflict");

        assert_eq!(err, SaveError::Conflict);
        // The concurrent writer's content survives.
        assert_eq!(
            std::fs::read_to_string(&plan).expect("read"),
            "claude wrote this"
        );
    }
}
