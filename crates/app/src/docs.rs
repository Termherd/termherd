//! Plans & memory discovery (`F-plans-memory`) — read-only for now. Lists the
//! plan files under `~/.claude/plans`, the global `~/.claude/CLAUDE.md`, and a
//! `CLAUDE.md` for each known project that has one, then reads a file on
//! demand. A file adapter owned by the shell, like [`crate::settings`]; we only
//! read here (editing, and the write-scope change it needs, is a later slice).

use std::path::{Path, PathBuf};

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

/// `~/.claude` — read-only home of plans and memory.
fn claude_home() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".claude"))
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
}
