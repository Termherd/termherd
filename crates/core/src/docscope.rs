//! Write-scope for plans & memory documents (`F-plans-memory` editing slice).
//! This is the security boundary for in-app editing: *which* document paths
//! termherd may write, and *whether* a save is safe when another writer (a
//! live Claude process) may have touched the file.
//!
//! Pure and I/O-free so it is exhaustively testable — the app adapter performs
//! the actual atomic write only after this module says yes. The allow-list is
//! deliberately narrow (ADR `0001`): the global memory file, plan files, and
//! project `CLAUDE.md`s; never session JSONL, never `~/.claude/ide`.

use std::path::Path;
use std::time::SystemTime;

/// Whether `path` is a document termherd is allowed to write, given the user's
/// `claude_home` (`~/.claude`).
///
/// Allowed:
/// - the global memory `<claude_home>/CLAUDE.md`;
/// - a plan file directly under `<claude_home>/plans/` with a `.md` extension;
/// - any `CLAUDE.md` that lives *outside* `claude_home` (a project memory file,
///   normal repo scope).
///
/// Denied (everything else), notably: session JSONL under
/// `<claude_home>/projects/`, anything under `<claude_home>/ide/`, plans nested
/// below `plans/`, non-`.md` files in `plans/`, and any non-`CLAUDE.md` file
/// outside `claude_home`.
#[must_use]
pub fn is_writable(path: &Path, claude_home: &Path) -> bool {
    match path.strip_prefix(claude_home) {
        // Inside ~/.claude: only the global memory file and direct plan files.
        Ok(relative) => {
            let segments: Vec<_> = relative.components().collect();
            match segments.as_slice() {
                [file] => file.as_os_str() == "CLAUDE.md",
                [dir, file] => dir.as_os_str() == "plans" && has_extension(file.as_os_str(), "md"),
                _ => false,
            }
        }
        // Outside ~/.claude: a project memory file (normal repo scope).
        Err(_) => path.file_name().is_some_and(|name| name == "CLAUDE.md"),
    }
}

/// Whether a single path component (a file name) carries the given extension.
fn has_extension(file_name: &std::ffi::OsStr, ext: &str) -> bool {
    Path::new(file_name)
        .extension()
        .is_some_and(|actual| actual == ext)
}

/// What to do when saving, given the file's mtime captured at open time and its
/// mtime on disk right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveDecision {
    /// The file is unchanged since it was opened; the write is safe.
    Proceed,
    /// The file changed on disk since it was opened (a concurrent writer, e.g.
    /// a live Claude process). Warn before overwriting — last-writer-wins here
    /// would be silent data loss.
    Conflict,
}

/// Decide whether a save may proceed. A `Conflict` arises only when the file is
/// strictly newer on disk than when it was opened; an equal or older on-disk
/// mtime is safe to overwrite.
#[must_use]
pub fn decide_save(open_mtime: SystemTime, on_disk_mtime: SystemTime) -> SaveDecision {
    if on_disk_mtime > open_mtime {
        SaveDecision::Conflict
    } else {
        SaveDecision::Proceed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    const HOME: &str = "/home/u/.claude";

    fn home() -> PathBuf {
        PathBuf::from(HOME)
    }

    // --- is_writable: allow-list -------------------------------------------

    #[test]
    fn global_memory_is_writable() {
        assert!(is_writable(&home().join("CLAUDE.md"), &home()));
    }

    #[test]
    fn plan_md_directly_under_plans_is_writable() {
        assert!(is_writable(
            &home().join("plans").join("atomic-purring-torvalds.md"),
            &home()
        ));
    }

    #[test]
    fn project_claude_md_outside_home_is_writable() {
        assert!(is_writable(Path::new("/repo/termherd/CLAUDE.md"), &home()));
    }

    // --- is_writable: deny-list --------------------------------------------

    #[test]
    fn session_jsonl_under_projects_is_denied() {
        assert!(!is_writable(
            &home().join("projects").join("-repo").join("abc.jsonl"),
            &home()
        ));
    }

    #[test]
    fn project_claude_md_under_home_is_denied() {
        // A CLAUDE.md that happens to live *inside* ~/.claude/projects is the
        // protected session tree, not a project memory file.
        assert!(!is_writable(
            &home().join("projects").join("-repo").join("CLAUDE.md"),
            &home()
        ));
    }

    #[test]
    fn ide_dir_is_denied() {
        assert!(!is_writable(&home().join("ide").join("lock.json"), &home()));
    }

    #[test]
    fn nested_plan_is_denied() {
        assert!(!is_writable(
            &home().join("plans").join("sub").join("x.md"),
            &home()
        ));
    }

    #[test]
    fn non_md_file_in_plans_is_denied() {
        assert!(!is_writable(
            &home().join("plans").join("notes.txt"),
            &home()
        ));
    }

    #[test]
    fn other_file_directly_under_home_is_denied() {
        assert!(!is_writable(&home().join("settings.json"), &home()));
        assert!(!is_writable(&home().join("README.md"), &home()));
    }

    #[test]
    fn arbitrary_file_outside_home_is_denied() {
        assert!(!is_writable(Path::new("/repo/src/main.rs"), &home()));
        assert!(!is_writable(Path::new("/etc/passwd"), &home()));
    }

    // --- decide_save -------------------------------------------------------

    #[test]
    fn unchanged_mtime_proceeds() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        assert_eq!(decide_save(t, t), SaveDecision::Proceed);
    }

    #[test]
    fn newer_on_disk_conflicts() {
        let open = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let on_disk = open + Duration::from_secs(5);
        assert_eq!(decide_save(open, on_disk), SaveDecision::Conflict);
    }

    #[test]
    fn older_on_disk_proceeds() {
        let open = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let on_disk = SystemTime::UNIX_EPOCH + Duration::from_secs(995);
        assert_eq!(decide_save(open, on_disk), SaveDecision::Proceed);
    }

    // --- property-based ----------------------------------------------------

    proptest::proptest! {
        /// Anything under `projects/` is never writable, whatever the name.
        #[test]
        fn projects_subtree_is_never_writable(
            seg in "[a-zA-Z0-9_.-]{1,30}",
            name in "[a-zA-Z0-9_.-]{1,30}",
        ) {
            let p = home().join("projects").join(&seg).join(&name);
            proptest::prop_assert!(!is_writable(&p, &home()));
        }

        /// Anything under `ide/` is never writable.
        #[test]
        fn ide_subtree_is_never_writable(name in "[a-zA-Z0-9_.-]{1,30}") {
            let p = home().join("ide").join(&name);
            proptest::prop_assert!(!is_writable(&p, &home()));
        }

        /// Every `.md` file placed directly in `plans/` is writable.
        #[test]
        fn direct_plan_md_is_always_writable(stem in "[a-zA-Z0-9_-]{1,30}") {
            let p = home().join("plans").join(format!("{stem}.md"));
            proptest::prop_assert!(is_writable(&p, &home()));
        }

        /// A file in `plans/` without a `.md` extension is never writable.
        #[test]
        fn non_md_in_plans_is_never_writable(
            stem in "[a-zA-Z0-9_-]{1,30}",
            ext in "(txt|json|jsonl|rs|toml)",
        ) {
            let p = home().join("plans").join(format!("{stem}.{ext}"));
            proptest::prop_assert!(!is_writable(&p, &home()));
        }

        /// `decide_save` is monotone in the on-disk mtime: a strictly newer
        /// on-disk file conflicts; equal or older proceeds.
        #[test]
        fn decide_save_is_monotone_in_on_disk_mtime(
            base in 0u64..1_000_000,
            delta in 1u64..1_000_000,
        ) {
            let open = SystemTime::UNIX_EPOCH + Duration::from_secs(base);
            let newer = open + Duration::from_secs(delta);
            let older = SystemTime::UNIX_EPOCH + Duration::from_secs(base.saturating_sub(delta));
            proptest::prop_assert_eq!(decide_save(open, open), SaveDecision::Proceed);
            proptest::prop_assert_eq!(decide_save(open, newer), SaveDecision::Conflict);
            proptest::prop_assert_eq!(decide_save(open, older), SaveDecision::Proceed);
        }
    }
}
