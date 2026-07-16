//! Spawn and activity policy: how a launch's command line and environment are
//! built, the private mcp-config file a Claude launch points `--mcp-config` at,
//! and how the OSC signal stream folds into the running activity status (FR8).
//! Pure of any real PTY, so the command/env contract and the status fold are
//! unit-tested directly.

use std::path::{Path, PathBuf};

use portable_pty::CommandBuilder;
use termherd_claude::osc::OscSignal;
use termherd_core::workspace::SessionId;
use termherd_core::{Launch, McpConfig, SessionStatus};

/// Fold a chunk's OSC signals into the running activity status (FR8).
///
/// Busy/idle titles track work; an OSC 9 notification means the CLI wants the
/// user (a permission prompt or an explicit ping) → [`SessionStatus::Attention`].
/// Attention is sticky: a plain idle prompt does not clear it (the user still
/// has to act); only real work resuming (`Busy`) does. Bells and alt-screen
/// toggles never change the activity status.
pub(crate) fn fold_status(current: SessionStatus, signals: &[OscSignal]) -> SessionStatus {
    let mut status = current;
    for signal in signals {
        status = match signal {
            OscSignal::Busy => SessionStatus::Busy,
            // A pending attention request outranks a bare idle prompt.
            OscSignal::Idle if status == SessionStatus::Attention => SessionStatus::Attention,
            OscSignal::Idle => SessionStatus::Idle,
            OscSignal::Notification(_) => SessionStatus::Attention,
            // The title text drives the tab label, not the status.
            OscSignal::Title(_) | OscSignal::AltScreen(_) | OscSignal::Bell => status,
        };
    }
    status
}

/// The line to type into the freshly spawned shell to start a [`Launch`], or
/// `None` for a plain shell (the bare shell *is* the deliverable). Typing keeps
/// `claude` resolution on the user's own shell + PATH, robust across platforms
/// (FR4a). `mcp_config`, when set, is the path to a written `mcpServers` file
/// passed as `--mcp-config` so the session can reach termherd's live bridge —
/// the path is on argv, but the token inside the file is not. Pure so the
/// command contract is unit-tested without a real PTY.
pub(crate) fn launch_command(launch: &Launch, mcp_config: Option<&Path>) -> Option<String> {
    let mcp_flag = mcp_config
        .map(|path| format!(" --mcp-config {}", path.display()))
        .unwrap_or_default();
    match launch {
        Launch::Shell => None,
        Launch::Claude { resume: None } => Some(format!("claude{mcp_flag}\r")),
        Launch::Claude { resume: Some(id) } => Some(format!("claude{mcp_flag} --resume {id}\r")),
    }
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
    fn a_plain_shell_launch_types_nothing() {
        assert_eq!(launch_command(&Launch::Shell, None), None);
    }

    #[test]
    fn a_fresh_claude_launch_types_bare_claude() {
        // The 🤖 button must start Claude *fresh*, never with
        // a stray `--resume`.
        assert_eq!(
            launch_command(&Launch::Claude { resume: None }, None),
            Some("claude\r".to_owned())
        );
    }

    #[test]
    fn a_claude_launch_with_an_mcp_config_passes_the_flag_before_resume() {
        let path = Path::new("/tmp/termherd-mcp-3.json");
        assert_eq!(
            launch_command(&Launch::Claude { resume: None }, Some(path)),
            Some("claude --mcp-config /tmp/termherd-mcp-3.json\r".to_owned()),
            "a fresh Claude gets the mcp flag"
        );
        assert_eq!(
            launch_command(
                &Launch::Claude {
                    resume: Some("abc-123".to_owned())
                },
                Some(path)
            ),
            Some("claude --mcp-config /tmp/termherd-mcp-3.json --resume abc-123\r".to_owned()),
            "the mcp flag precedes --resume"
        );
    }

    #[test]
    fn a_resumed_claude_launch_types_resume_with_the_id() {
        assert_eq!(
            launch_command(
                &Launch::Claude {
                    resume: Some("abc-123".to_owned())
                },
                None
            ),
            Some("claude --resume abc-123\r".to_owned())
        );
    }

    #[test]
    fn fold_status_tracks_busy_idle_attention() {
        use SessionStatus::*;
        // The last busy/idle marker in the chunk wins.
        assert_eq!(
            fold_status(Starting, &[OscSignal::Busy, OscSignal::Idle]),
            Idle
        );
        assert_eq!(fold_status(Idle, &[OscSignal::Busy]), Busy);
        // An OSC 9 notification means the CLI needs the user → Attention.
        assert_eq!(
            fold_status(Busy, &[OscSignal::Notification("x".into())]),
            Attention
        );
        // Attention is sticky against a bare idle prompt, but Busy clears it.
        assert_eq!(fold_status(Attention, &[OscSignal::Idle]), Attention);
        assert_eq!(fold_status(Attention, &[OscSignal::Busy]), Busy);
        // Bells and alt-screen toggles leave the status unchanged.
        assert_eq!(
            fold_status(Busy, &[OscSignal::Bell, OscSignal::AltScreen(true)]),
            Busy
        );
        // No signals at all keeps the current status (e.g. a plain shell).
        assert_eq!(fold_status(Starting, &[]), Starting);
    }

    #[test]
    fn notification_still_means_attention_alongside_the_osc9_body_forwarding() {
        use SessionStatus::*;
        // The notification *text* is forwarded to the OS on a separate channel;
        // the status fold must be untouched — an OSC 9 among other signals
        // still resolves to Attention, and its body never leaks into the title.
        let signals = [
            OscSignal::Busy,
            OscSignal::Notification("permission: allow Bash?".into()),
            OscSignal::Title("ignored".into()),
        ];
        assert_eq!(fold_status(Starting, &signals), Attention);
        // An empty-bodied notification is just as much an attention request.
        assert_eq!(
            fold_status(Idle, &[OscSignal::Notification(String::new())]),
            Attention
        );
    }
}
