//! Spawn policy: how a launch's command line and environment are built, and the
//! private files a Claude launch points at — the mcp-config behind
//! `--mcp-config` and the settings overlay behind `--settings`. Pure of any real
//! PTY, so the command/env contract is unit-tested directly.

use std::path::{Path, PathBuf};

use portable_pty::CommandBuilder;
use termherd_core::workspace::SessionId;
use termherd_core::{Launch, McpConfig};

use crate::integration::integration_for;

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
    let dir = std::env::temp_dir().join(format!("termherd-shell-{}", session.0.get()));
    // zsh's own replay source is `$ZDOTDIR`, falling back to `$HOME` — read it
    // *before* the recipe overwrites `ZDOTDIR` for the child.
    let home = std::env::var_os("ZDOTDIR")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    let Some(integration) = integration_for(program, &dir, home.as_deref()) else {
        return;
    };
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
    let _ = std::fs::remove_dir_all(temp.join(format!("termherd-shell-{id}")));
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

    #[test]
    fn terminal_env_advertises_iterm2_for_status_osc() {
        // Claude only emits its status OSC stream under iTerm2, so the
        // spawned command must claim that identity — otherwise every activity
        // indicator stays frozen on the launch status.
        let mut cmd = CommandBuilder::new("/bin/sh");
        apply_terminal_env(&mut cmd);
        assert_eq!(
            cmd.get_env("TERM_PROGRAM"),
            Some(std::ffi::OsStr::new("iTerm.app"))
        );
        assert!(
            cmd.get_env("TERM_PROGRAM_VERSION").is_some(),
            "a version must accompany TERM_PROGRAM for any version gating"
        );
        assert_eq!(
            cmd.get_env("TERM"),
            Some(std::ffi::OsStr::new("xterm-256color"))
        );
    }

    #[test]
    fn performing_a_recipe_writes_its_files_and_points_the_command_at_them() {
        // The recipe being right is worth nothing if it is never performed:
        // this is the seam between the tested contract and the spawned command.
        let session = SessionId(std::num::NonZeroU64::new(97).expect("nonzero"));
        let mut cmd = CommandBuilder::new("/bin/zsh");
        apply_integration(session, "/bin/zsh", &mut cmd);

        let zdotdir = cmd
            .get_env("ZDOTDIR")
            .and_then(|value| value.to_str())
            .map(PathBuf::from)
            .expect("the spawned shell is pointed at the private directory");
        let rc = std::fs::read_to_string(zdotdir.join(".zshrc")).expect("the rc was written");
        assert!(
            rc.contains("133;A"),
            "the written rc carries the hooks, got {rc}"
        );
        let _ = std::fs::remove_dir_all(&zdotdir);
    }

    #[test]
    fn a_directory_left_by_a_crashed_run_does_not_strand_the_next_one() {
        // Session ids restart at 1 with each run, so the second run's session 1
        // meets the first run's directory. Refusing it there would cost that
        // session its marks for good — a bug that only shows up after a crash.
        let session = SessionId(std::num::NonZeroU64::new(96).expect("nonzero"));
        let stale = std::env::temp_dir().join("termherd-shell-96");
        std::fs::create_dir_all(&stale).expect("leave a stale directory");
        std::fs::write(stale.join(".zshrc"), "stale\n").expect("with stale contents");

        let mut cmd = CommandBuilder::new("/bin/zsh");
        apply_integration(session, "/bin/zsh", &mut cmd);
        assert!(
            cmd.get_env("ZDOTDIR").is_some(),
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
        let dir = std::env::temp_dir().join("termherd-shell-95");
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
    fn performing_an_unknown_shells_recipe_leaves_the_command_alone() {
        let session = SessionId(std::num::NonZeroU64::new(98).expect("nonzero"));
        let mut cmd = CommandBuilder::new("/usr/bin/nu");
        apply_integration(session, "/usr/bin/nu", &mut cmd);
        assert_eq!(cmd.get_env("ZDOTDIR"), None);
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
