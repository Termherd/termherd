//! Shell integration: what a spawned shell needs so it reports its prompt and
//! command boundaries as OSC 133 marks (`crate::prompt`), and the directory it
//! is in as an OSC 7 url (`crate::workdir`).
//!
//! Both ride the same prompt hook, because both answer a question asked at the
//! same moment: what is this shell doing, and where. The directory is written
//! from the shell's own live `$PWD` — a path baked in when the file is
//! generated would report the launch directory forever, which is the bug the
//! announcement exists to fix.
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

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The name every private per-session directory a recipe is materialised under
/// begins with. `crate::launch` builds the directory from it and
/// [`is_generated_shell_dir`] recognises one by it, so the name a nested launch
/// has to see through is the name it was given.
pub(crate) const SHELL_DIR_PREFIX: &str = "termherd-shell-";

/// Where the zsh recipe records the directory it displaced, so a termherd
/// launched from the shell it starts can still find the user's own.
///
/// `ZDOTDIR` alone cannot say this: by the time the nested instance reads it,
/// it holds the private directory the outer one installed. The variable is
/// inherited like any other, so it crosses however many ordinary processes sit
/// between the two — only the recipe that displaces `ZDOTDIR` has to set it.
const HANDOFF: &str = "TERMHERD_ORIG_ZDOTDIR";

/// The environment variables a shell's replay source is read from, in
/// preference order.
const REPLAY_SOURCES: [&str; 3] = [HANDOFF, "ZDOTDIR", "HOME"];

/// Where the user's own startup files live — the directory the generated ones
/// replay from ([`replay`]), read through `lookup` so the choice is testable
/// without touching the process environment.
///
/// zsh's own source is `$ZDOTDIR` before `$HOME`; every other dialect reads
/// `$HOME` alone, and neither of their recipes displaces it. The [`HANDOFF`]
/// comes first because it is the only candidate that stays right across a
/// nesting level.
///
/// **A directory termherd generated is never one of them.** A termherd launched
/// from a termherd shell inherits a `ZDOTDIR` pointing at that shell's private
/// files; replaying from there makes the new session's `.zshenv` source itself
/// when the session ids collide — the shell then never reaches a prompt — and
/// chain another session's hooks when they do not. Refusing it is what covers
/// the cases the handoff cannot reach: a directory left by an older termherd,
/// or one whose own `$HOME` was unreadable. Nothing left to replay is the
/// documented degradation ([`replay`]); replaying the wrong thing is not.
pub(crate) fn replay_home(lookup: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    REPLAY_SOURCES
        .iter()
        .filter_map(|key| lookup(key))
        .map(PathBuf::from)
        .find(|candidate| !is_generated_shell_dir(candidate))
}

/// Whether `path` is a private per-session directory termherd wrote.
///
/// Recognised by its **name**, not by the temp directory it sits in: a nested
/// instance can run under a different `TMPDIR` than the one that exported the
/// variable, so the parent proves nothing about who wrote it.
pub(crate) fn is_generated_shell_dir(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with(SHELL_DIR_PREFIX))
}

/// What a shell needs to report itself. Pure data: nothing here touches the
/// filesystem or a `CommandBuilder`.
///
/// The path is handed to `printf` as an argument rather than spliced into the
/// format string, so a `%` in a directory name is not read as a format
/// directive — and it is escaped to `%25` on the way out, so the decoder cannot
/// mistake a real `/tmp/100%20` for a path with a space in it. That escape is
/// the *only* one: every other character rides raw, because the decoder passes
/// through what it cannot read as an escape, and a shell has no url encoder to
/// reach for. Escaping the one ambiguous character is what makes the round trip
/// exact rather than merely forgiving.
///
/// zsh and bash spell it `${PWD//\%/%25}` — the backslash is load-bearing:
/// unescaped, zsh reads the pattern as an anchor and appends `%25` to the path
/// instead of replacing anything, which bash does not. Verified against both,
/// since no `cargo check` sees inside a generated shell string.
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
    let (_, recipe) = RECIPES.iter().find(|(name, _)| *name == shell)?;
    Some(recipe(dir, home))
}

/// How one dialect is instrumented, given its private directory and the user's
/// home. `fish` needs neither — it is instrumented inline — and takes them to
/// keep the table one shape.
type Recipe = fn(&Path, Option<&Path>) -> Integration;

/// Every shell dialect termherd knows how to instrument. The dispatch above and
/// the tests below both read *this* list, so a dialect added here without hooks
/// fails the sweep rather than being missed by a hand-written enumeration of
/// the shells someone remembered.
const RECIPES: &[(&str, Recipe)] = &[("zsh", zsh), ("bash", bash), ("fish", fish)];

/// zsh reads every startup file from `ZDOTDIR`, so pointing it at a private
/// directory is the way in — and the reason each generated file must replay the
/// user's counterpart, which taking `ZDOTDIR` over has displaced.
///
/// It is also the reason this recipe exports [`HANDOFF`]: the shell it starts
/// carries the displaced `ZDOTDIR` in its environment, and a termherd launched
/// from that shell would otherwise read it as the user's own. Nothing is
/// exported when there is no home to record — an empty variable is still
/// *found* by the next resolver, which would then replay from `/`.
fn zsh(dir: &Path, home: Option<&Path>) -> Integration {
    let hooks = "\
termherd_precmd() { printf '\\033]133;D\\007\\033]133;A\\007\\033]7;file://%s%s\\007' \"$HOST\" \"${PWD//\\%/%25}\" }\n\
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
    let mut env = vec![("ZDOTDIR".to_owned(), dir.display().to_string())];
    if let Some(home) = home {
        env.push((HANDOFF.to_owned(), home.display().to_string()));
    }
    Integration {
        files,
        env,
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
        "termherd_precmd() { printf '\\033]133;D\\007\\033]133;A\\007\\033]7;file://%s%s\\007' \"$HOSTNAME\" \"${PWD//\\%/%25}\"; }\n\
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
fn fish(_dir: &Path, _home: Option<&Path>) -> Integration {
    let init = "\
function termherd_prompt --on-event fish_prompt; printf '\\033]133;D\\007\\033]133;A\\007\\033]7;file://%s%s\\007' \"$hostname\" (string replace -a '%' '%25' -- \"$PWD\"); end; \
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
///
/// The separator is a literal `/`, not the host's: this string is a line of
/// POSIX shell, so the separator belongs to the *script's* grammar rather than
/// to the filesystem the script was generated on. `Path::join` would write a
/// backslash when the generator runs on Windows, where the shell reads it as an
/// escape and sources nothing.
fn replay(home: Option<&Path>, name: &str) -> String {
    let Some(home) = home else {
        return String::new();
    };
    let home = home.display().to_string();
    let path = format!("{}/{name}", home.trim_end_matches(['/', '\\']));
    format!("if [ -f \"{path}\" ]; then . \"{path}\"; fi\n")
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// The private per-session directory a recipe is materialised under.
    fn dir() -> &'static Path {
        Path::new("/tmp/termherd-shell-7")
    }
    fn home() -> Option<&'static Path> {
        Some(Path::new("/Users/someone"))
    }

    /// The variable the zsh recipe is expected to export so the user's own
    /// directory survives a nested launch. Spelled out rather than shared with
    /// the code under test: it is a contract with shells already running, so a
    /// rename must fail here rather than pass silently on both sides.
    const EXPECTED_HANDOFF: &str = "TERMHERD_ORIG_ZDOTDIR";

    /// A stand-in process environment for [`replay_home`].
    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key| {
            pairs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| OsString::from(v))
        }
    }

    /// A directory termherd generated, as an inherited variable would carry it.
    const GENERATED: &str = "/var/folders/s0/T/termherd-shell-33";

    #[test]
    fn a_zdotdir_termherd_installed_is_never_the_replay_source() {
        // The whole bug: a termherd launched from a termherd shell inherits a
        // ZDOTDIR pointing at generated files. Replaying from there makes the
        // new session's `.zshenv` source *itself* when the session ids collide
        // — the shell then never starts — and chain another session's hooks
        // when they do not. Either way the user's own rc is never reached.
        assert_eq!(
            replay_home(env_of(&[
                ("ZDOTDIR", GENERATED),
                ("HOME", "/Users/someone")
            ]))
            .as_deref(),
            Some(Path::new("/Users/someone")),
        );
    }

    #[test]
    fn a_zdotdir_the_user_set_themselves_is_still_the_replay_source() {
        // Skipping every ZDOTDIR would be a cure worse than the disease:
        // someone whose zsh configuration lives outside $HOME would silently
        // lose all of it the moment termherd opened a shell.
        assert_eq!(
            replay_home(env_of(&[
                ("ZDOTDIR", "/Users/someone/.config/zsh"),
                ("HOME", "/Users/someone"),
            ]))
            .as_deref(),
            Some(Path::new("/Users/someone/.config/zsh")),
        );
    }

    #[test]
    fn the_handoff_outranks_a_zdotdir_of_either_kind() {
        // What carries the user's own directory across a nesting level: the
        // parent exported it beside the private ZDOTDIR that displaced it, so
        // it is read first — and it is the only candidate that can still be
        // right when the ZDOTDIR beside it is one termherd wrote.
        assert_eq!(
            replay_home(env_of(&[
                (EXPECTED_HANDOFF, "/Users/someone/.config/zsh"),
                ("ZDOTDIR", GENERATED),
                ("HOME", "/Users/someone"),
            ]))
            .as_deref(),
            Some(Path::new("/Users/someone/.config/zsh")),
        );
    }

    #[test]
    fn a_handoff_that_is_itself_generated_is_refused_like_any_other() {
        // Nothing about being read first makes a value trustworthy — the
        // variable crosses process boundaries and anything can set it. The
        // refusal is a property of the *directory*, applied to every candidate.
        assert_eq!(
            replay_home(env_of(&[
                (EXPECTED_HANDOFF, GENERATED),
                ("ZDOTDIR", GENERATED),
                ("HOME", "/Users/someone"),
            ]))
            .as_deref(),
            Some(Path::new("/Users/someone")),
        );
    }

    #[test]
    fn a_generated_directory_is_recognised_by_its_name_not_by_where_it_sits() {
        // An inner instance can run under a different TMPDIR than the one that
        // exported the variable, so the parent directory proves nothing about
        // who wrote it. Only the name termherd itself chose does.
        for elsewhere in [
            "/var/folders/s0/T/termherd-shell-33",
            "/tmp/claude-501/termherd-shell-2",
            "/tmp/termherd-shell-7",
        ] {
            assert_eq!(
                replay_home(env_of(&[
                    ("ZDOTDIR", elsewhere),
                    ("HOME", "/Users/someone")
                ]))
                .as_deref(),
                Some(Path::new("/Users/someone")),
                "{elsewhere} is one of ours wherever it sits",
            );
        }
        // And a directory that merely reads like one is the user's business.
        for theirs in [
            "/Users/someone/termherd-shellish",
            "/Users/someone/termherd-shell",
            "/Users/someone/.config/termherd",
        ] {
            assert_eq!(
                replay_home(env_of(&[("ZDOTDIR", theirs), ("HOME", "/Users/someone")])).as_deref(),
                Some(Path::new(theirs)),
                "{theirs} is not ours to refuse",
            );
        }
    }

    #[test]
    fn nothing_to_replay_beats_replaying_from_a_generated_directory() {
        // No candidate left means no `source` line at all, which is the
        // degradation `replay` already documents: the user loses their prompt
        // and aliases. Sourcing the generated files instead loses the shell.
        assert_eq!(
            replay_home(env_of(&[
                (EXPECTED_HANDOFF, GENERATED),
                ("ZDOTDIR", GENERATED)
            ])),
            None,
        );
        assert_eq!(replay_home(env_of(&[])), None);
    }

    #[test]
    fn zsh_hands_the_users_own_directory_to_whatever_it_launches() {
        // The private ZDOTDIR this recipe exports is exactly what a termherd
        // launched from the resulting shell would inherit and replay from. The
        // handoff beside it is how that instance still finds the real one.
        let it = zsh(dir(), home());
        assert_eq!(
            it.env
                .iter()
                .find(|(key, _)| key == EXPECTED_HANDOFF)
                .map(|(_, value)| value.as_str()),
            Some("/Users/someone"),
            "got {:?}",
            it.env,
        );
        assert_eq!(
            it.env
                .iter()
                .find(|(key, _)| key == "ZDOTDIR")
                .map(|(_, value)| value.as_str()),
            Some("/tmp/termherd-shell-7"),
            "the handoff is set beside the private directory, not instead of it",
        );
    }

    #[test]
    fn zsh_hands_on_nothing_rather_than_an_empty_directory() {
        // An exported empty variable is still *found* by the next resolver,
        // which would then replay from `/` — worse than the ZDOTDIR it was
        // meant to correct, and silently so.
        let it = zsh(dir(), None);
        assert!(
            it.env.iter().all(|(key, _)| key != EXPECTED_HANDOFF),
            "got {:?}",
            it.env,
        );
    }

    #[test]
    fn a_generated_startup_file_never_sources_itself() {
        // The catastrophic half of the report, spelled as what the user sees:
        // when the inherited ZDOTDIR and the directory this session is about to
        // write are the *same* path — which restarting session ids make likely
        // rather than rare — every generated file replays itself, and the shell
        // dies on `job table full or recursion limit exceeded` without ever
        // reaching a prompt.
        let dir = Path::new("/var/folders/s0/T/termherd-shell-1");
        let home = replay_home(env_of(&[
            ("ZDOTDIR", "/var/folders/s0/T/termherd-shell-1"),
            ("HOME", "/Users/someone"),
        ]));
        for (path, contents) in zsh(dir, home.as_deref()).files {
            assert!(
                !contents.contains(&path.display().to_string()),
                "{} sources itself: {contents}",
                path.display(),
            );
        }
    }

    /// One nesting level: the environment a shell launched by this level sees,
    /// given the environment its termherd inherited. The parent's variables
    /// survive except where the recipe overrides them — which is what makes a
    /// stale `ZDOTDIR` reach the next instance in the first place.
    fn launched_from(env: &[(String, String)], dir: &Path) -> Vec<(String, String)> {
        let home = replay_home(|key| {
            env.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| OsString::from(v))
        });
        let mut env = env.to_vec();
        for (key, value) in zsh(dir, home.as_deref()).env {
            match env.iter_mut().find(|(k, _)| *k == key) {
                Some(entry) => entry.1 = value,
                None => env.push((key, value)),
            }
        }
        env
    }

    proptest! {
        /// The reported scenario, played to arbitrary depth: each termherd is
        /// launched from a shell the previous one opened, so it inherits that
        /// shell's environment. Session ids restart at 1 every run, so the
        /// generated directories collide across levels — the ticket's worst
        /// case, drawn from a range small enough that they usually do.
        #[test]
        fn the_users_own_directory_survives_any_depth_of_nesting(
            depth in 1usize..7,
            ids in prop::collection::vec(1u64..4, 7),
            temps in prop::collection::vec(0usize..3, 7),
            zdotdir_of_their_own in any::<bool>(),
        ) {
            let temp_dirs = ["/var/folders/s0/T", "/tmp", "/tmp/claude-501"];
            let theirs = if zdotdir_of_their_own {
                "/Users/someone/.config/zsh"
            } else {
                "/Users/someone"
            };
            let mut env = vec![("HOME".to_owned(), "/Users/someone".to_owned())];
            if zdotdir_of_their_own {
                env.push(("ZDOTDIR".to_owned(), theirs.to_owned()));
            }

            for level in 0..depth {
                let dir = Path::new(temp_dirs[temps[level]])
                    .join(format!("{SHELL_DIR_PREFIX}{}", ids[level]));
                let resolved = replay_home(|key| {
                    env.iter().find(|(k, _)| k == key).map(|(_, v)| OsString::from(v))
                });
                prop_assert_eq!(
                    resolved.as_deref(),
                    Some(Path::new(theirs)),
                    "level {} replayed from the wrong directory",
                    level,
                );
                env = launched_from(&env, &dir);
            }
        }
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

    /// Everything a dialect's recipe puts in front of the shell — every file it
    /// writes and every argument it appends — as one string to assert on. The
    /// three sweeps below walk [`RECIPES`] itself, so a dialect added to the
    /// table without hooks fails them rather than being missed by a list of the
    /// shells someone remembered to type.
    /// The url a snippet announces: what sits between the `]7;` introducer and
    /// the BEL the shell will print (`\007`, two literal characters here).
    fn announced_url(snippet: &str) -> &str {
        snippet
            .split("]7;")
            .nth(1)
            .expect("the recipe announces a directory")
            .split("\\007")
            .next()
            .expect("split always yields a first part")
    }

    fn instrumentation(recipe: Recipe) -> String {
        let it = recipe(dir(), home());
        let written: String = it.files.iter().map(|(_, c)| c.as_str()).collect();
        format!("{written}{}", it.args.join(" "))
    }

    #[test]
    fn every_recipe_makes_the_shell_announce_its_directory() {
        // Without OSC 7 a session's directory is the one it launched in,
        // frozen — so `PaneSnapshot.cwd` misreports from the first `cd`, and
        // a split inherits a directory the user left long ago.
        for (shell, recipe) in RECIPES {
            let snippet = instrumentation(*recipe);
            assert!(
                snippet.contains("]7;file://"),
                "{shell} must announce its directory, got {snippet}"
            );
        }
    }

    #[test]
    fn every_recipe_reads_the_directory_at_each_prompt_not_at_startup() {
        // The point is following a `cd`: a path baked in when the file is
        // generated would report the launch directory forever, which is the
        // very bug this closes. The shell's own live variable is the fix — read
        // through an expansion whose spelling differs per dialect, so the
        // assertion is on the variable rather than on one way of reading it.
        for (shell, recipe) in RECIPES {
            let snippet = instrumentation(*recipe);
            assert!(
                snippet.contains("PWD"),
                "{shell} must announce PWD, got {snippet}"
            );
            assert_eq!(
                announced_url(&snippet),
                "file://%s%s",
                "{shell} must announce host and path as printf arguments, so no \
                 directory can be baked into the url itself"
            );
        }
    }

    #[test]
    fn every_recipe_escapes_the_one_character_the_decoder_could_misread() {
        // A directory really called `100%20` would otherwise be announced
        // literally and come back with a space in it — the decoder inventing a
        // path rather than merely tolerating an odd one. `%` is the only
        // character with that property, so it is the only one escaped.
        for (shell, recipe) in RECIPES {
            let snippet = instrumentation(*recipe);
            assert!(
                snippet.contains("%25"),
                "{shell} must escape a literal % in the path, got {snippet}"
            );
        }
    }

    #[test]
    fn every_announcement_is_written_in_the_url_separator_not_the_hosts() {
        // The snippet is a line of POSIX shell producing a `file://` url: both
        // grammars separate with `/`, whatever the host that generated the file
        // uses. A `\` reaching either one breaks both — the replay fix's
        // lesson, applied to a second generated string.
        for (shell, recipe) in RECIPES {
            let snippet = instrumentation(*recipe);
            let url = announced_url(&snippet);
            assert!(
                url.starts_with("file://") && !url.contains('\\'),
                "{shell} must announce a url carrying no backslash, got {url}"
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
    fn the_replay_joins_with_one_separator_whatever_the_home_ends_with() {
        // The separator is written by hand rather than by `Path::join`, since
        // the line is POSIX shell and must not carry the host's separator. A
        // hand-written join owns the case `join` used to handle: a home that
        // already ends in one, which would otherwise source `//.zshrc`.
        for home in ["/Users/someone", "/Users/someone/"] {
            let rc = file_named(
                &integration_for("/bin/zsh", dir(), Some(Path::new(home)))
                    .expect("zsh has a recipe"),
                ".zshrc",
            );
            assert!(
                rc.contains("\"/Users/someone/.zshrc\""),
                "home {home:?} must source one path, got {rc}"
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
