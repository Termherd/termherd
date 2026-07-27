//! Shell integration: what a spawned shell needs so it reports its prompt and
//! command boundaries as OSC 133 marks (`crate::prompt`).
//!
//! No shell does this on its own out of the box, which is why a plain shell had
//! no activity at all. Every integrated terminal — iTerm2, VS Code, WezTerm —
//! solves it the same way: hand the shell a startup file that first loads the
//! user's own configuration, then appends the hooks that emit the marks. The
//! *recipe* is pure data here (files to write, environment to set, arguments to
//! append) so the whole per-shell contract is unit-tested without spawning
//! anything; `crate::launch::apply_integration` performs it, where the
//! private-file discipline the written startup files need already lives.
//!
//! Sourcing the user's own startup file first is the load-bearing part: taking
//! over `ZDOTDIR` without replaying what it displaced would silently drop the
//! user's prompt, aliases and PATH — a far worse bug than the one being fixed.

use std::path::{Path, PathBuf};

/// What a shell needs to emit OSC 133 marks. Pure data: nothing here touches
/// the filesystem or a `CommandBuilder`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Integration {
    /// Files to write, as (absolute path, contents).
    pub files: Vec<(PathBuf, String)>,
    /// Environment variables to set on the spawned command.
    pub env: Vec<(String, String)>,
    /// Arguments to append to the spawned command.
    pub args: Vec<String>,
}

/// The integration recipe for `program`, materialised under `dir` (a private
/// per-session directory), or `None` for a shell termherd has no recipe for —
/// which is not a failure: that session simply falls back on the foreground
/// process group (`crate::status::foreground_status`).
///
/// `home` is where the user's own startup files live, replayed by the generated
/// ones; `None` replays nothing (see [`replay`]). The hooks themselves never
/// depend on it.
pub(crate) fn integration_for(
    program: &str,
    dir: &Path,
    home: Option<&Path>,
) -> Option<Integration> {
    // The configured shell may be an absolute path, a bare name, or (on
    // Windows) carry an extension; all that identifies the dialect is the stem.
    let shell = Path::new(program).file_stem()?.to_str()?;
    match shell {
        "zsh" => Some(zsh(dir, home)),
        "bash" => Some(bash(dir, home)),
        "fish" => Some(fish()),
        _ => None,
    }
}

/// zsh reads every startup file from `ZDOTDIR`, so pointing it at a private
/// directory is the way in — and the reason each generated file must replay the
/// user's counterpart, which taking `ZDOTDIR` over has displaced.
fn zsh(dir: &Path, home: Option<&Path>) -> Integration {
    let hooks = "\
termherd_precmd() { printf '\\033]133;D\\007\\033]133;A\\007' }\n\
termherd_preexec() { printf '\\033]133;C\\007' }\n\
autoload -Uz add-zsh-hook\n\
add-zsh-hook precmd termherd_precmd\n\
add-zsh-hook preexec termherd_preexec\n";
    // The hooks belong in `.zshrc` alone — it is the interactive file, and the
    // others would install them in non-interactive shells too.
    let files = [".zshenv", ".zprofile", ".zshrc", ".zlogin"]
        .into_iter()
        .map(|name| {
            let mut contents = replay(home, name);
            if name == ".zshrc" {
                contents.push_str(hooks);
            }
            (dir.join(name), contents)
        })
        .collect();
    Integration {
        files,
        env: vec![("ZDOTDIR".to_owned(), dir.display().to_string())],
        args: Vec::new(),
    }
}

/// bash takes its startup file as an argument, so nothing of the user's is
/// displaced by an environment variable — but `--rcfile` *replaces* the one it
/// would have read, so the replay is still needed.
fn bash(dir: &Path, home: Option<&Path>) -> Integration {
    let rcfile = dir.join("termherd.bash");
    let mut contents = replay(home, ".bashrc");
    // `PROMPT_COMMAND` runs before each prompt; the DEBUG trap fires before
    // each command. Both are appended, never assigned, so a user who set them
    // in their own rc keeps theirs.
    contents.push_str(
        "termherd_precmd() { printf '\\033]133;D\\007\\033]133;A\\007'; }\n\
         termherd_preexec() { printf '\\033]133;C\\007'; }\n\
         PROMPT_COMMAND=\"termherd_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}\"\n\
         trap 'termherd_preexec' DEBUG\n",
    );
    Integration {
        files: vec![(rcfile.clone(), contents)],
        env: Vec::new(),
        args: vec!["--rcfile".to_owned(), rcfile.display().to_string()],
    }
}

/// fish runs `--init-command` *after* its own configuration, so nothing is
/// displaced and no file is needed: the hooks go inline.
fn fish() -> Integration {
    let init = "\
function termherd_prompt --on-event fish_prompt; printf '\\033]133;D\\007\\033]133;A\\007'; end; \
function termherd_preexec --on-event fish_preexec; printf '\\033]133;C\\007'; end";
    Integration {
        files: Vec::new(),
        env: Vec::new(),
        args: vec!["--init-command".to_owned(), init.to_owned()],
    }
}

/// The line that loads the user's own `name` startup file, guarded on it
/// existing — a fresh account has none, and an unguarded `source` would print an
/// error into the first line of every terminal termherd opens.
///
/// The `if` form rather than `[ -f x ] && . x`: the latter *evaluates to 1* when
/// the file is absent, and the last startup file read leaves that status behind
/// for the shell's first command. A bare `exit` inherits it, so the session ends
/// unclean and its tab does not auto-close — an integration that quietly changes
/// what exiting means. The path is quoted, so a home directory containing a
/// space is sourced rather than mis-parsed into `[: too many arguments`.
///
/// No home means **no line at all**. Falling back to the bare `name` would make
/// it relative, and the session's working directory is the *project* — so the
/// shell would source whatever `.zshrc` a cloned repository happens to carry.
fn replay(home: Option<&Path>, name: &str) -> String {
    let Some(home) = home else {
        return String::new();
    };
    let path = home.join(name).display().to_string();
    format!("if [ -f \"{path}\" ]; then . \"{path}\"; fi\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The private per-session directory a recipe is materialised under.
    fn dir() -> &'static Path {
        Path::new("/tmp/termherd-shell-7")
    }
    fn home() -> Option<&'static Path> {
        Some(Path::new("/Users/someone"))
    }

    /// The single file a recipe writes, by suffix — every recipe here writes at
    /// least one startup file, and the tests care about its contents.
    fn file_named(integration: &Integration, name: &str) -> String {
        integration
            .files
            .iter()
            .find(|(path, _)| path.file_name().is_some_and(|f| f == name))
            .map(|(_, contents)| contents.clone())
            .unwrap_or_else(|| panic!("no {name} among {:?}", integration.files))
    }

    #[test]
    fn zsh_is_pointed_at_a_private_zdotdir() {
        let it = integration_for("/bin/zsh", dir(), home()).expect("zsh has a recipe");
        assert_eq!(
            it.env
                .iter()
                .find(|(key, _)| key == "ZDOTDIR")
                .map(|(_, value)| value.as_str()),
            Some("/tmp/termherd-shell-7"),
            "zsh reads its startup files from ZDOTDIR, so that is the way in"
        );
        assert!(
            it.args.is_empty(),
            "zsh needs no extra argument, only the env"
        );
    }

    #[test]
    fn the_zsh_startup_file_emits_all_three_marks() {
        let rc = file_named(
            &integration_for("/bin/zsh", dir(), home()).expect("zsh has a recipe"),
            ".zshrc",
        );
        assert!(
            rc.contains("precmd") && rc.contains("preexec"),
            "the marks ride on zsh's own prompt hooks, got {rc}"
        );
        for mark in ["133;A", "133;C", "133;D"] {
            assert!(rc.contains(mark), "the snippet must emit {mark}, got {rc}");
        }
    }

    #[test]
    fn the_zsh_startup_file_replays_the_users_own() {
        // Taking over ZDOTDIR displaces every file zsh would have read. Each
        // generated one must load its counterpart first, or the user loses
        // their prompt, aliases and PATH the moment termherd launches a shell.
        let it = integration_for("/bin/zsh", dir(), home()).expect("zsh has a recipe");
        for name in [".zshrc", ".zshenv", ".zprofile", ".zlogin"] {
            let contents = file_named(&it, name);
            assert!(
                contents.contains(&format!("/Users/someone/{name}")),
                "{name} must source the user's own, got {contents}"
            );
        }
    }

    #[test]
    fn the_zsh_startup_file_survives_a_user_who_has_none() {
        // A fresh account has no ~/.zshrc; sourcing it unguarded would print an
        // error into the very first line of every terminal termherd opens.
        let rc = file_named(
            &integration_for("/bin/zsh", dir(), home()).expect("zsh has a recipe"),
            ".zshrc",
        );
        assert!(
            rc.contains("[ -f") || rc.contains("[[ -f"),
            "the replay must be guarded on the file existing, got {rc}"
        );
    }

    #[test]
    fn the_replay_leaves_no_failure_status_behind() {
        // `[ -f x ] && . x` evaluates to 1 when x is absent, and the last
        // startup file read hands that status to the shell's first command: a
        // bare `exit` then ends the session unclean and its tab never
        // auto-closes. Guarding must not change what exiting means.
        let it = integration_for("/bin/zsh", dir(), home()).expect("zsh has a recipe");
        for (path, contents) in &it.files {
            assert!(
                !contents.contains("] && ."),
                "{} must guard with `if`, not a status-carrying `&&`: {contents}",
                path.display()
            );
        }
    }

    #[test]
    fn bash_is_pointed_at_a_private_rcfile() {
        let it = integration_for("/bin/bash", dir(), home()).expect("bash has a recipe");
        let rcfile = it
            .args
            .windows(2)
            .find(|pair| pair[0] == "--rcfile")
            .map(|pair| pair[1].clone())
            .expect("bash takes its startup file as an argument");
        assert!(
            rcfile.starts_with("/tmp/termherd-shell-7"),
            "the rcfile lives in the session's private directory, got {rcfile}"
        );
        let rc = file_named(&it, "termherd.bash");
        assert!(
            rc.contains("PROMPT_COMMAND") && rc.contains("133;A"),
            "the marks ride on bash's prompt hook, got {rc}"
        );
        assert!(
            rc.contains("/Users/someone/.bashrc"),
            "the user's own rc must still load, got {rc}"
        );
    }

    #[test]
    fn fish_gets_its_hooks_as_an_init_command() {
        // fish has no rcfile flag; `--init-command` runs after its own config,
        // so nothing of the user's is displaced in the first place.
        let it = integration_for("/usr/local/bin/fish", dir(), home()).expect("fish has a recipe");
        let init = it
            .args
            .windows(2)
            .find(|pair| pair[0] == "--init-command")
            .map(|pair| pair[1].clone())
            .expect("fish takes its hooks inline");
        assert!(
            init.contains("fish_preexec") && init.contains("fish_prompt"),
            "the marks ride on fish's own events, got {init}"
        );
        assert!(
            it.files.is_empty(),
            "an inline init command needs no file on disk"
        );
    }

    #[test]
    fn a_shell_is_recognised_by_its_name_not_its_path() {
        // The configured shell may be an absolute path, a bare name, or a
        // versioned build; all of them are still zsh.
        for program in ["zsh", "/bin/zsh", "/opt/homebrew/bin/zsh"] {
            assert!(
                integration_for(program, dir(), home()).is_some(),
                "{program} should be recognised as zsh"
            );
        }
    }

    #[test]
    fn an_unknown_shell_gets_no_recipe_rather_than_a_broken_one() {
        // Guessing would corrupt the startup of a shell whose grammar we do not
        // know. No recipe simply means the foreground poll stands in.
        assert_eq!(integration_for("/usr/bin/nu", dir(), home()), None);
        assert_eq!(integration_for("pwsh", dir(), home()), None);
    }

    #[test]
    fn a_recipe_without_a_known_home_still_produces_working_hooks() {
        // `home` only drives the replay; the marks must not depend on it.
        let it = integration_for("/bin/zsh", dir(), None).expect("zsh has a recipe");
        let rc = file_named(&it, ".zshrc");
        assert!(
            rc.contains("133;A"),
            "the hooks are unconditional, got {rc}"
        );
    }

    #[test]
    fn a_recipe_without_a_known_home_sources_nothing_at_all() {
        // A bare `.zshrc` would be *relative*, and the session's working
        // directory is the project — so the shell would source whatever a
        // cloned repository happens to carry. Nothing to replay means no line.
        let it = integration_for("/bin/zsh", dir(), None).expect("zsh has a recipe");
        for (path, contents) in &it.files {
            assert!(
                !contents.contains(". ."),
                "{} must source nothing, got {contents}",
                path.display()
            );
        }
    }

    #[test]
    fn the_replay_quotes_the_path_it_sources() {
        // An unquoted path with a space in it makes `[ -f a b ]` fail with
        // "too many arguments": the user's own rc is then silently never
        // loaded, and the error prints on the first line of every terminal.
        let spaced = Path::new("/Users/some one");
        let rc = file_named(
            &integration_for("/bin/zsh", dir(), Some(spaced)).expect("zsh has a recipe"),
            ".zshrc",
        );
        assert!(
            rc.contains("\"/Users/some one/.zshrc\""),
            "the sourced path must be quoted, got {rc}"
        );
    }
}
