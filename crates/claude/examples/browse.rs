//! Dev demo: a text preview of the future session browser (M1), running
//! the pure codec over the real `~/.claude/projects` tree.
//!
//! This is a development tool, not product code — the folder walking and
//! fs checks done inline here are the future `scan` adapter's job, and the
//! grouping logic previewed here will live in `core`.
//!
//! Run with: `cargo run -p termherd-claude --example browse`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use termherd_claude::derive::{collapse_worktree, extract_cwd};
use termherd_claude::digest::{SessionDigest, digest_session};

struct SessionRow {
    digest: SessionDigest,
    modified: Option<std::time::SystemTime>,
}

fn main() {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    let projects_dir: PathBuf = [home.as_str(), ".claude", "projects"].iter().collect();

    // project path → sessions (BTreeMap for a stable, sorted display)
    let mut groups: BTreeMap<String, Vec<SessionRow>> = BTreeMap::new();
    let mut skipped = 0u32;

    let Ok(folders) = std::fs::read_dir(&projects_dir) else {
        println!(
            "Pas de dossier {} sur cette machine.",
            projects_dir.display()
        );
        return;
    };

    for folder in folders.flatten() {
        let dir = folder.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().is_none_or(|e| e != "jsonl") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(digest) = digest_session(&content) else {
                skipped += 1;
                continue;
            };
            let project = extract_cwd(&content)
                .map(|cwd| group_path(&cwd))
                .unwrap_or_else(|| format!("<non dérivé: {}>", dir.display()));
            let modified = file.metadata().and_then(|m| m.modified()).ok();
            groups
                .entry(project)
                .or_default()
                .push(SessionRow { digest, modified });
        }
    }

    let total: usize = groups.values().map(Vec::len).sum();
    println!("TermHerd — aperçu du navigateur de sessions (codec M1)");
    println!(
        "{} projet(s), {} session(s), {} sans prompt réel (ignorées)\n",
        groups.len(),
        total,
        skipped
    );

    for (project, mut sessions) in groups {
        sessions.sort_by_key(|row| std::cmp::Reverse(row.modified));
        println!("▸ {project}");
        for row in &sessions {
            let d = &row.digest;
            let age = row.modified.map(age_label).unwrap_or_default();
            println!(
                "    {:<52} {:>4} msgs  {}",
                clip(d.display_title(None), 52),
                d.message_count,
                age,
            );
        }
        println!();
    }
}

/// The grouping key the sidebar will use: the real cwd, with worktrees
/// collapsed onto their main checkout when it still exists on disk —
/// the fs check upstream does in `resolveWorktreePath`.
fn group_path(cwd: &str) -> String {
    match collapse_worktree(cwd) {
        Some(parent) if Path::new(parent).exists() => parent.to_owned(),
        _ => cwd.to_owned(),
    }
}

fn clip(s: &str, max: usize) -> String {
    let cleaned: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        let mut out: String = cleaned.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn age_label(t: std::time::SystemTime) -> String {
    let Ok(elapsed) = t.elapsed() else {
        return String::new();
    };
    let secs = elapsed.as_secs();
    match secs {
        0..=3599 => format!("il y a {} min", secs / 60),
        3600..=86_399 => format!("il y a {} h", secs / 3600),
        _ => format!("il y a {} j", secs / 86_400),
    }
}
