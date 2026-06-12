//! Real-project-path derivation — the inverse of the lossy folder encoding
//! in [`crate::path`].
//!
//! Folder names under `~/.claude/projects/` cannot be decoded (every
//! non-alphanumeric becomes `-`, so `/a-b` and `/a/b` collide). The real
//! path is recovered from the `cwd` field Claude CLI writes into each JSONL
//! line — the trick behind the duplicate-sidebar bug class (#41/#44).
//!
//! Ported from `derive-project-path.js` (upstream Electron app,
//! `doctly/switchboard`). Only the pure parts live here: walking a project
//! folder (direct `*.jsonl` first, then UUID subdirectories and their
//! `subagents/`) is the scan adapter's job, as is checking that a worktree
//! parent actually exists before collapsing onto it.

/// Extract the first `cwd` value from JSONL content.
///
/// Mirrors `extractCwdFromJsonl`: scan entries in order (line policy in
/// [`crate::jsonl`]) and return the first non-empty string `cwd`. (The JS
/// reference accepts any truthy `cwd`; in practice the CLI always writes a
/// string, so non-strings are skipped here.)
pub fn extract_cwd(jsonl: &str) -> Option<String> {
    crate::jsonl::entries(jsonl).find_map(|value| {
        let cwd = value.get("cwd")?.as_str()?;
        if cwd.is_empty() {
            None
        } else {
            Some(cwd.to_owned())
        }
    })
}

/// If `cwd` points inside a worktree checkout, return the main project path
/// it should be grouped under.
///
/// Mirrors `resolveWorktreePath`: the last path component is the worktree
/// name and the component(s) before it must be one of the worktree markers
/// (`.claude/worktrees`, `.claude-worktrees`, `.worktrees`). One trailing
/// separator is tolerated. Returns `None` when `cwd` is not a worktree
/// path, or when the would-be parent is empty.
///
/// The upstream check `fs.existsSync(parent)` is **not** done here — this
/// crate is pure. Callers must verify the returned candidate exists and
/// fall back to the original `cwd` if it does not.
///
/// Upstream is macOS-only and matches `/` separators; TermHerd also accepts
/// pure-`\` Windows paths (mixed separators are not recognised).
pub fn collapse_worktree(cwd: &str) -> Option<&str> {
    collapse_with(
        cwd,
        '/',
        &["/.claude/worktrees", "/.claude-worktrees", "/.worktrees"],
    )
    .or_else(|| {
        collapse_with(
            cwd,
            '\\',
            &[
                "\\.claude\\worktrees",
                "\\.claude-worktrees",
                "\\.worktrees",
            ],
        )
    })
}

fn collapse_with<'a>(cwd: &'a str, sep: char, markers: &[&str]) -> Option<&'a str> {
    let trimmed = cwd.strip_suffix(sep).unwrap_or(cwd);
    // The worktree name is the final component and may not be empty.
    let (rest, name) = trimmed.rsplit_once(sep)?;
    if name.is_empty() {
        return None;
    }
    for marker in markers {
        if let Some(parent) = rest.strip_suffix(marker)
            && !parent.is_empty()
        {
            return Some(parent);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn first_cwd_wins() {
        let jsonl = r#"{"type":"summary"}
{"cwd":"/Users/a/proj","type":"user"}
{"cwd":"/Users/a/other","type":"user"}"#;
        assert_eq!(extract_cwd(jsonl).as_deref(), Some("/Users/a/proj"));
    }

    #[test]
    fn bad_json_lines_are_skipped() {
        let jsonl = "not json at all\n{\"cwd\":\"/p\"}";
        assert_eq!(extract_cwd(jsonl).as_deref(), Some("/p"));
    }

    #[test]
    fn empty_and_non_string_cwd_are_skipped() {
        let jsonl = r#"{"cwd":""}
{"cwd":42}
{"cwd":"/real"}"#;
        assert_eq!(extract_cwd(jsonl).as_deref(), Some("/real"));
    }

    #[test]
    fn no_cwd_yields_none() {
        assert_eq!(extract_cwd(""), None);
        assert_eq!(extract_cwd("{\"type\":\"user\"}"), None);
    }

    #[test]
    fn crlf_lines_parse() {
        let jsonl = "{\"type\":\"x\"}\r\n{\"cwd\":\"/p\"}\r\n";
        assert_eq!(extract_cwd(jsonl).as_deref(), Some("/p"));
    }

    #[test]
    fn collapses_each_marker() {
        assert_eq!(
            collapse_worktree("/Users/a/proj/.worktrees/feat"),
            Some("/Users/a/proj")
        );
        assert_eq!(
            collapse_worktree("/Users/a/proj/.claude-worktrees/feat"),
            Some("/Users/a/proj")
        );
        assert_eq!(
            collapse_worktree("/Users/a/proj/.claude/worktrees/feat"),
            Some("/Users/a/proj")
        );
    }

    #[test]
    fn tolerates_one_trailing_slash() {
        assert_eq!(
            collapse_worktree("/Users/a/proj/.worktrees/feat/"),
            Some("/Users/a/proj")
        );
        // Two trailing slashes leave an empty final component → not a match.
        assert_eq!(collapse_worktree("/Users/a/proj/.worktrees/feat//"), None);
    }

    #[test]
    fn non_worktree_paths_pass_through() {
        assert_eq!(collapse_worktree("/Users/a/proj"), None);
        assert_eq!(collapse_worktree("/Users/a/worktrees/feat"), None);
        // A path *inside* a worktree (extra component) is not collapsed —
        // the marker must be the second-to-last component.
        assert_eq!(collapse_worktree("/p/.worktrees/feat/sub"), None);
    }

    #[test]
    fn empty_parent_is_not_a_match() {
        assert_eq!(collapse_worktree("/.worktrees/feat"), None);
    }

    #[test]
    fn windows_separators_collapse() {
        assert_eq!(
            collapse_worktree(r"C:\projets\proj\.worktrees\feat"),
            Some(r"C:\projets\proj")
        );
        assert_eq!(
            collapse_worktree(r"C:\projets\proj\.claude\worktrees\feat"),
            Some(r"C:\projets\proj")
        );
    }

    proptest! {
        #[test]
        fn extract_cwd_roundtrips_arbitrary_paths(cwd in "\\PC+") {
            let line = serde_json::json!({ "cwd": cwd, "type": "user" });
            let jsonl = format!("{line}\n");
            // serde_json never emits raw newlines inside a string, so the
            // line parses back and the first cwd must be ours.
            prop_assert_eq!(extract_cwd(&jsonl), Some(cwd));
        }

        #[test]
        fn extract_cwd_never_panics(input in any::<String>()) {
            let _ = extract_cwd(&input);
        }

        #[test]
        fn collapse_recovers_the_parent(
            parent in "/[a-zA-Z0-9_./-]{1,40}[a-zA-Z0-9]",
            name in "[a-zA-Z0-9_-]{1,20}",
        ) {
            let cwd = format!("{parent}/.worktrees/{name}");
            prop_assert_eq!(collapse_worktree(&cwd), Some(parent.as_str()));
        }

        #[test]
        fn collapse_never_panics(input in any::<String>()) {
            let _ = collapse_worktree(&input);
        }
    }
}
