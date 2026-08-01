//! Spawn policy: how a launch's command line and environment are built, and the
//! private files a Claude launch points at — the mcp-config behind
//! `--mcp-config` and the settings overlay behind `--settings`. Pure of any real
//! PTY, so the command/env contract is unit-tested directly.

use std::path::{Path, PathBuf};

use portable_pty::CommandBuilder;
use termherd_core::workspace::SessionId;
use termherd_core::{Launch, McpConfig};

use crate::integration::{SHELL_DIR_PREFIX, integration_for, replay_home};

/// The line to type into the freshly spawned shell to start a [`Launch`], or
/// `None` for a plain shell (the bare shell *is* the deliverable). Typing keeps
/// `claude` resolution on the user's own shell + PATH, robust across platforms
/// (FR4a). `mcp_config`, when set, is the path to a written `mcpServers` file
/// passed as `--mcp-config` so the session can reach termherd's live bridge —
/// the path is on argv, but the token inside the file is not. `settings` is the
/// [`write_title_settings`] overlay that keeps the status channel open. Pure so
/// the command contract is unit-tested without a real PTY.
pub(crate) fn launch_command(
    launch: &Launch,
    mcp_config: Option<&Path>,
    settings: Option<&Path>,
) -> Option<String> {
    let flags = [
        mcp_config.map(|path| format!(" --mcp-config {}", path.display())),
        settings.map(|path| format!(" --settings {}", path.display())),
    ]
    .into_iter()
    .flatten()
    .collect::<String>();
    match launch {
        Launch::Shell => None,
        Launch::Claude { resume: None } => Some(format!("claude{flags}\r")),
        Launch::Claude { resume: Some(id) } => Some(format!("claude{flags} --resume {id}\r")),
    }
}

/// Write the settings overlay a Claude launch passes to `--settings`, and return
/// its path.
///
/// termherd derives every Claude session's activity from the CLI's OSC 0 title
/// (see [`apply_terminal_env`]), and `CLAUDE_CODE_DISABLE_TERMINAL_TITLE` turns
/// that title off. A user who sets it in `~/.claude/settings.json` silences
/// termherd's only status channel — and that `env` block outranks the
/// environment we spawn with, so exporting the variable back to `0` does
/// nothing. Only a `--settings` overlay outranks it in turn; it *merges* with
/// the user's settings rather than replacing them, so nothing else they
/// configured is lost. `None` (logged, not fatal) if the write fails: the
/// session then launches with whatever title setting the user has.
///
/// `--settings` arrived in Claude Code **1.0.61**, which is therefore the CLI
/// floor termherd's README states: an older one rejects the flag and the launch
/// fails outright, rather than merely losing its status.
pub(crate) fn write_title_settings(session: SessionId) -> Option<PathBuf> {
    let path = std::env::temp_dir().join(format!("termherd-settings-{}.json", session.0.get()));
    let json = r#"{"env":{"CLAUDE_CODE_DISABLE_TERMINAL_TITLE":"0"}}"#;
    match write_private(&path, json) {
        Ok(()) => Some(path),
        Err(error) => {
            tracing::warn!(
                %error,
                "failed to write the settings overlay; the session's status may stay `starting`"
            );
            None
        }
    }
}

/// Perform the shell-integration recipe for `program` on a command about to be
/// spawned: write its startup files into a private per-session directory, then
/// apply the environment and arguments that point the shell at them.
///
/// The impure half of [`crate::integration`], deliberately thin — everything it
/// *decides* lives there, pure and unit-tested. It lives in this module because
/// the files are secrets-adjacent in the same sense as the mcp config: they are
/// executed by the user's shell, so they go through the same private-write
/// discipline (see [`private_dir`]).
///
/// A shell with no recipe, or a directory that cannot be made privately, leaves
/// the command untouched: that session then falls back on the foreground process
/// group ([`crate::status::foreground_status`]). A degradation, not a failure,
/// so it is logged rather than propagated.
pub(crate) fn apply_integration(session: SessionId, program: &str, cmd: &mut CommandBuilder) {
    let dir = shell_dir(session);
    // Read *before* the recipe overwrites `ZDOTDIR` for the child.
    let home = replay_home(|key| std::env::var_os(key));
    let Some(integration) = integration_for(program, &dir, home.as_deref()) else {
        return;
    };
    // A recipe carrying arguments cannot be applied to the platform's *default*
    // program: `portable_pty` marks that builder by its empty argv, so the first
    // argument both turns it into an ordinary command and drops the `-basename`
    // argv0 that makes it a **login shell** — silently changing which startup
    // files the user's shell reads. Wanting a status is not a reason to change
    // how someone's shell starts, so the recipe is declined and the foreground
    // poll stands in. A shell the user configured explicitly already carries its
    // own argv, so it takes the recipe.
    if !integration.args.is_empty() && cmd.is_default_prog() {
        tracing::debug!(
            session = session.0.get(),
            program,
            "shell integration needs arguments the default login shell cannot take; \
             this session's activity falls back on its foreground process"
        );
        return;
    }
    if let Err(error) = write_startup_files(&dir, &integration.files) {
        tracing::warn!(
            %error,
            session = session.0.get(),
            "could not write the shell integration; \
             this session's activity falls back on its foreground process"
        );
        return;
    }
    for (key, value) in &integration.env {
        cmd.env(key, value);
    }
    for arg in &integration.args {
        cmd.arg(arg);
    }
}

/// The private per-session directory a shell-integration recipe is written
/// into. Named once, because three readers need the same answer: the launch
/// that writes it, the teardown that removes it, and — through
/// [`SHELL_DIR_PREFIX`] — the nested launch that must not mistake it for the
/// user's own home.
fn shell_dir(session: SessionId) -> PathBuf {
    std::env::temp_dir().join(format!("{SHELL_DIR_PREFIX}{}", session.0.get()))
}

/// Create the private directory and write every startup file into it.
fn write_startup_files(dir: &Path, files: &[(PathBuf, String)]) -> std::io::Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    // Session ids restart at 1 with each run, so a directory left behind by a
    // crashed one would collide — and [`private_dir`] refuses to adopt what it
    // finds, which would cost that session its marks *for good*. Clearing first
    // keeps that refusal aimed at directories we could not have written: in a
    // shared temp directory another user's entry cannot be removed either.
    let _ = std::fs::remove_dir_all(dir);
    private_dir(dir)?;
    for (path, contents) in files {
        write_private(path, contents)?;
    }
    Ok(())
}

/// Delete every private file this session was given: its shell-integration
/// directory, its mcp config, its settings overlay. Called when the session is
/// torn down.
///
/// Without it each run leaves its temp files behind for good — and the mcp
/// config holds a **bearer token** for termherd's live bridge, which has no
/// business outliving the session it authorised. Best-effort by construction:
/// a file already gone is the desired state, and a teardown is no place to fail.
pub(crate) fn discard_private_files(session: SessionId) {
    let temp = std::env::temp_dir();
    let id = session.0.get();
    let _ = std::fs::remove_dir_all(shell_dir(session));
    let _ = std::fs::remove_file(temp.join(format!("termherd-mcp-{id}.json")));
    let _ = std::fs::remove_file(temp.join(format!("termherd-settings-{id}.json")));
}

/// Create `path` as a directory only this user can reach (`0o700` on Unix), and
/// **fail if it already exists**.
///
/// Both halves are load-bearing. The default temp directory is per-user on
/// macOS but is the shared, world-writable `/tmp` on Linux, and what goes in
/// here is not merely read — it is *sourced by the user's login shell*. A
/// directory another local user pre-created under the name we are about to use
/// would let them choose what that shell executes. Refusing to adopt an existing
/// directory turns that into a fallback to the foreground poll; the private mode
/// keeps the files unreadable and unwritable to anyone else afterwards.
#[cfg(unix)]
fn private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    std::fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}

/// Write the `mcpServers` config for a Claude launch and return its path, so
/// `launch_command` can point `--mcp-config` at it. The file — not argv — holds
/// the bearer token. Both fields are known-safe for a bare JSON string (a
/// loopback url and a hex token), so no escaping is needed. `None` (logged, not
/// fatal) if the write fails: the session then launches without the live bridge.
pub(crate) fn write_mcp_config(session: SessionId, config: &McpConfig) -> Option<PathBuf> {
    let path = std::env::temp_dir().join(format!("termherd-mcp-{}.json", session.0.get()));
    let json = format!(
        r#"{{"mcpServers":{{"termherd":{{"type":"http","url":"{}","headers":{{"Authorization":"Bearer {}"}}}}}}}}"#,
        config.url, config.token
    );
    match write_private(&path, &json) {
        Ok(()) => Some(path),
        Err(error) => {
            tracing::warn!(
                %error,
                "failed to write mcp config; session launches without the live bridge"
            );
            None
        }
    }
}

/// Write `contents` to `path` readable only by the current user (`0o600` on
/// Unix) — the file carries a bearer token, so a world-readable temp file would
/// leak it to any other local user. On non-Unix the platform's per-user temp
/// ACLs are relied on.
#[cfg(unix)]
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents.as_bytes())
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    std::fs::write(path, contents)
}

/// What [`apply_terminal_env`] advertises as the host terminal. Claude CLI only
/// emits its status / notification OSC sequences (the busy / idle / attention
/// signals `termherd_claude::osc` decodes, FR8) when it believes it is running
/// under iTerm2 — it sniffs `TERM_PROGRAM`. Without this the status stays on
/// whatever it was at launch (`Starting`), which is the "tab status stuck" bug.
const TERM_PROGRAM: &str = "iTerm.app";
/// A recent iTerm2 version, so any minimum-version gating on the CLI side also
/// passes. The exact value only has to read as "new enough".
const TERM_PROGRAM_VERSION: &str = "3.5.0";

/// Set the environment a Claude session expects: a colour-capable `TERM`, and
/// the iTerm2 identity that unlocks its OSC status stream. Kept separate
/// from `PtyManager::spawn` so the env contract is unit-testable without a
/// real PTY.
pub(crate) fn apply_terminal_env(cmd: &mut CommandBuilder) {
    cmd.env("TERM", "xterm-256color");
    cmd.env("TERM_PROGRAM", TERM_PROGRAM);
    cmd.env("TERM_PROGRAM_VERSION", TERM_PROGRAM_VERSION);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three flags a fully-wired Claude launch carries, as paths.
    fn mcp() -> &'static Path {
        Path::new("/tmp/termherd-mcp-3.json")
    }
    fn settings() -> &'static Path {
        Path::new("/tmp/termherd-settings-3.json")
    }

    /// What a launch *set* on the builder, as opposed to what the builder
    /// merely reports: `get_env` answers from the inherited process environment
    /// when nothing overrode it. Every variable these tests are about — TERM,
    /// TERM_PROGRAM, ZDOTDIR — is one a termherd session exports to its own
    /// shells, so a test run from inside termherd finds them all there and an
    /// assertion that nothing was set passes for the wrong reason. Reading only
    /// the builder's own overrides is what makes the claim independent of where
    /// the test was launched from.
    fn set_on(cmd: &CommandBuilder, key: &str) -> Option<String> {
        cmd.iter_extra_env_as_str()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.to_owned())
    }

    #[test]
    fn terminal_env_advertises_iterm2_for_status_osc() {
        // Claude only emits its status OSC stream under iTerm2, so the
        // spawned command must claim that identity — otherwise every activity
        // indicator stays frozen on the launch status.
        let mut cmd = CommandBuilder::new("/bin/sh");
        apply_terminal_env(&mut cmd);
        assert_eq!(set_on(&cmd, "TERM_PROGRAM").as_deref(), Some("iTerm.app"));
        assert!(
            set_on(&cmd, "TERM_PROGRAM_VERSION").is_some(),
            "a version must accompany TERM_PROGRAM for any version gating"
        );
        assert_eq!(set_on(&cmd, "TERM").as_deref(), Some("xterm-256color"));
    }

    #[test]
    fn performing_a_recipe_writes_its_files_and_points_the_command_at_them() {
        // The recipe being right is worth nothing if it is never performed:
        // this is the seam between the tested contract and the spawned command.
        let session = SessionId(std::num::NonZeroU64::new(97).expect("nonzero"));
        let mut cmd = CommandBuilder::new("/bin/zsh");
        apply_integration(session, "/bin/zsh", &mut cmd);

        let dir = shell_dir(session);
        assert_eq!(
            set_on(&cmd, "ZDOTDIR"),
            Some(dir.display().to_string()),
            "the spawned shell is pointed at its own private directory"
        );
        let rc = std::fs::read_to_string(dir.join(".zshrc")).expect("the rc was written");
        assert!(
            rc.contains("133;A"),
            "the written rc carries the hooks, got {rc}"
        );
        discard_private_files(session);
    }

    #[test]
    fn a_directory_left_by_a_crashed_run_does_not_strand_the_next_one() {
        // Session ids restart at 1 with each run, so the second run's session 1
        // meets the first run's directory. Refusing it there would cost that
        // session its marks for good — a bug that only shows up after a crash.
        let session = SessionId(std::num::NonZeroU64::new(96).expect("nonzero"));
        let stale = shell_dir(session);
        std::fs::create_dir_all(&stale).expect("leave a stale directory");
        std::fs::write(stale.join(".zshrc"), "stale\n").expect("with stale contents");

        let mut cmd = CommandBuilder::new("/bin/zsh");
        apply_integration(session, "/bin/zsh", &mut cmd);
        assert_eq!(
            set_on(&cmd, "ZDOTDIR"),
            Some(stale.display().to_string()),
            "our own leftovers must be cleared, not treated as hostile"
        );
        let rc = std::fs::read_to_string(stale.join(".zshrc")).expect("rewritten");
        assert!(rc.contains("133;A"), "and rewritten, got {rc}");
        discard_private_files(session);
    }

    #[test]
    fn tearing_a_session_down_takes_its_private_files_with_it() {
        // The mcp config holds the live bridge's bearer token; leaving it in a
        // temp directory after the session is gone leaves the token there too.
        let session = SessionId(std::num::NonZeroU64::new(95).expect("nonzero"));
        let mut cmd = CommandBuilder::new("/bin/zsh");
        apply_integration(session, "/bin/zsh", &mut cmd);
        let overlay = write_title_settings(session).expect("overlay written");
        let dir = shell_dir(session);
        assert!(dir.exists() && overlay.exists(), "both exist while it runs");

        discard_private_files(session);
        assert!(!dir.exists(), "the integration directory is gone");
        assert!(!overlay.exists(), "the settings overlay is gone");
    }

    #[test]
    fn a_directory_that_is_already_there_is_never_adopted() {
        // The startup files are *sourced by the user's login shell*, and the
        // default temp directory is the shared `/tmp` on Linux. Adopting a
        // directory already in place would let whoever created it choose what
        // that shell runs. Our own leftovers are cleared first
        // (`write_startup_files`); what survives that clearing is, by
        // construction, a directory another user owns — and it must be refused.
        let planted = std::env::temp_dir().join("termherd-planted-94");
        let _ = std::fs::remove_dir_all(&planted);
        std::fs::create_dir_all(&planted).expect("plant the directory");

        assert!(
            private_dir(&planted).is_err(),
            "an existing directory must never be adopted"
        );
        let _ = std::fs::remove_dir_all(&planted);
    }

    #[test]
    fn a_recipe_needing_arguments_is_declined_for_the_default_login_shell() {
        // `portable_pty` marks the platform default program by its empty argv,
        // and panics on the first argument added to it — so bash's `--rcfile`
        // recipe would have crashed every session on a machine whose default
        // shell is bash, which is most Linux ones. Even without the panic, the
        // argument would demote the login shell to an ordinary one and change
        // which startup files run. Declining costs the marks, not the session.
        let session = SessionId(std::num::NonZeroU64::new(93).expect("nonzero"));
        // Cleared first: this is the one test that asserts an *absence* under
        // the shared temp directory, so a directory left by an earlier failed
        // — or mutated — run would fail it for reasons that are not its own.
        discard_private_files(session);
        let mut cmd = CommandBuilder::new_default_prog();
        apply_integration(session, "/bin/bash", &mut cmd);
        assert!(
            cmd.is_default_prog(),
            "the default login shell must be left exactly as it was"
        );
        assert!(
            !shell_dir(session).exists(),
            "and nothing is written for a recipe that will not be applied"
        );
    }

    #[test]
    fn an_explicitly_configured_shell_still_takes_an_arguments_recipe() {
        // The same recipe is fine on a shell the user named: that builder
        // already carries its own argv, so there is no login-shell argv0 to
        // lose and no panic to trip.
        let session = SessionId(std::num::NonZeroU64::new(92).expect("nonzero"));
        let mut cmd = CommandBuilder::new("/bin/bash");
        apply_integration(session, "/bin/bash", &mut cmd);
        assert!(
            cmd.get_argv().iter().any(|arg| arg == "--rcfile"),
            "got {:?}",
            cmd.get_argv()
        );
        discard_private_files(session);
    }

    #[test]
    fn a_nested_run_does_not_replay_from_the_directory_it_inherited() {
        // The only assertion that crosses the real `std::env` wiring, and it
        // can only run where the bug exists: a test process launched from a
        // termherd shell, whose environment carries a generated ZDOTDIR. CI
        // inherits none, so there it asserts nothing — which is the whole
        // claim, not a caveat on a broader one. The rule itself is proved
        // against a stand-in environment in `crate::integration`.
        let Some(inherited) = std::env::var_os("ZDOTDIR").map(PathBuf::from) else {
            return;
        };
        if !crate::integration::is_generated_shell_dir(&inherited) {
            return;
        }
        assert_ne!(
            replay_home(|key| std::env::var_os(key)),
            Some(inherited),
            "a run nested inside a termherd shell must reach past what it inherited"
        );
    }

    #[test]
    fn performing_an_unknown_shells_recipe_leaves_the_command_alone() {
        let session = SessionId(std::num::NonZeroU64::new(98).expect("nonzero"));
        let mut cmd = CommandBuilder::new("/usr/bin/nu");
        apply_integration(session, "/usr/bin/nu", &mut cmd);
        assert_eq!(set_on(&cmd, "ZDOTDIR"), None);
        assert!(cmd.get_argv().len() <= 1, "no arguments were appended");
    }

    #[test]
    fn a_plain_shell_launch_types_nothing() {
        assert_eq!(launch_command(&Launch::Shell, None, None), None);
    }

    #[test]
    fn a_fresh_claude_launch_types_bare_claude() {
        // The 🤖 button must start Claude *fresh*, never with
        // a stray `--resume`.
        assert_eq!(
            launch_command(&Launch::Claude { resume: None }, None, None),
            Some("claude\r".to_owned())
        );
    }

    #[test]
    fn a_claude_launch_with_an_mcp_config_passes_the_flag_before_resume() {
        assert_eq!(
            launch_command(&Launch::Claude { resume: None }, Some(mcp()), None),
            Some("claude --mcp-config /tmp/termherd-mcp-3.json\r".to_owned()),
            "a fresh Claude gets the mcp flag"
        );
        assert_eq!(
            launch_command(
                &Launch::Claude {
                    resume: Some("abc-123".to_owned())
                },
                Some(mcp()),
                None
            ),
            Some("claude --mcp-config /tmp/termherd-mcp-3.json --resume abc-123\r".to_owned()),
            "the mcp flag precedes --resume"
        );
    }

    #[test]
    fn a_claude_launch_passes_the_settings_overlay_before_resume() {
        // The overlay is what keeps the OSC title — and therefore the whole
        // status channel — alive against a user who disabled it globally.
        assert_eq!(
            launch_command(&Launch::Claude { resume: None }, None, Some(settings())),
            Some("claude --settings /tmp/termherd-settings-3.json\r".to_owned())
        );
        assert_eq!(
            launch_command(
                &Launch::Claude {
                    resume: Some("abc-123".to_owned())
                },
                Some(mcp()),
                Some(settings())
            ),
            Some(
                concat!(
                    "claude --mcp-config /tmp/termherd-mcp-3.json",
                    " --settings /tmp/termherd-settings-3.json --resume abc-123\r"
                )
                .to_owned()
            ),
            "both flags precede --resume"
        );
    }

    #[test]
    fn the_settings_overlay_re_enables_the_terminal_title() {
        // The one claim the file exists for: whatever the user's own settings
        // say, a termherd-launched session reports its status.
        let path = write_title_settings(SessionId(std::num::NonZeroU64::new(41).expect("nonzero")))
            .expect("the overlay is written");
        let written = std::fs::read_to_string(&path).expect("readable overlay");
        assert!(
            written.contains(r#""CLAUDE_CODE_DISABLE_TERMINAL_TITLE":"0""#),
            "the overlay must turn the title back on, got {written}"
        );
        assert!(
            written.starts_with(r#"{"env":"#),
            "it must land in the env block Claude reads, got {written}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_resumed_claude_launch_types_resume_with_the_id() {
        assert_eq!(
            launch_command(
                &Launch::Claude {
                    resume: Some("abc-123".to_owned())
                },
                None,
                None
            ),
            Some("claude --resume abc-123\r".to_owned())
        );
    }
}
