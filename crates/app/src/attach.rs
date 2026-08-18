//! Publishing the live bridge, so a client termherd did **not** spawn can reach
//! it.
//!
//! Every other rung of the control surface assumes the caller is a Claude
//! session termherd launched: that is how it holds an endpoint url and a bearer
//! token at all, both handed to it through a `--mcp-config` file only its own
//! process tree is given. Nothing about the server refuses an outside client —
//! [`crate::mcp::Tokens`] accepts any token it minted, whoever presents it. What
//! an outside client lacks is *discovery*.
//!
//! This module is that discovery, and the consent gate in front of it. A running
//! instance may publish `{ url, token }` to a private file beside the rest of
//! its state; `termherd mcp-config` reads that file back and renders the
//! `mcpServers` snippet a client registers. The capability is unchanged — the
//! same tools, reached by a client that was not spawned as a child.
//!
//! Publishing is **off unless asked for**. What it exposes is not modest:
//! `run_in_session` types into a terminal, which is arbitrary command execution
//! as the user. That is already reachable by anything that can read a spawned
//! session's config file, so this makes it *discoverable* by default rather than
//! *newly possible* — an argument for stating the exposure, not for skipping the
//! gate.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file a running instance publishes itself into, under `~/.termherd`. One
/// instance runs at a time (the single-instance lock), so "which instance?"
/// needs no field in it.
const BRIDGE_FILE: &str = "bridge.json";

/// The environment variable that overrides the setting for one launch.
pub(crate) const ATTACH_ENV: &str = "TERMHERD_MCP_ATTACH";

/// What the user asked for about publishing the bridge — the `mcp.attach`
/// setting, as three states rather than two.
///
/// `Never` is not a third spelling of "off": it is off *and* beyond the reach of
/// [`ATTACH_ENV`], so a machine can refuse the exposure in a way an ambient
/// environment variable cannot quietly undo.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum AttachMode {
    /// Publish nothing; an override may turn this on.
    #[default]
    Off,
    /// Publish; an override may turn this off.
    On,
    /// Publish nothing, and refuse every override.
    Never,
}

/// The `mcp.attach` value as `settings.json` spells it: `true`, `false`, or the
/// string `"never"`. Anything else falls back to the default with a warning,
/// rather than failing serde for the whole file and silently resetting the
/// user's keymap, theme and shell along with it — the contract every tolerant
/// key here keeps.
impl<'de> Deserialize<'de> for AttachMode {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(de)?;
        if let Some(flag) = value.as_bool() {
            return Ok(if flag {
                AttachMode::On
            } else {
                AttachMode::Off
            });
        }
        if value.as_str() == Some(NEVER) {
            return Ok(AttachMode::Never);
        }
        // The safe state, not the last one that parsed: an unreadable value must
        // never be the reason the bridge is reachable.
        tracing::warn!(%value, "mcp.attach is not true/false/\"never\"; leaving it off");
        Ok(AttachMode::Off)
    }
}

impl Serialize for AttachMode {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        match self {
            AttachMode::Off => ser.serialize_bool(false),
            AttachMode::On => ser.serialize_bool(true),
            AttachMode::Never => ser.serialize_str(NEVER),
        }
    }
}

/// How `settings.json` spells the third state.
const NEVER: &str = "never";

/// What reconciling the setting with the environment decided, and whether an
/// override was refused on the way — the caller logs the refusal, so a user who
/// set the variable and saw nothing happen learns why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Verdict {
    /// Whether this launch publishes its endpoint.
    pub(crate) publish: bool,
    /// An override asked for the opposite of `Never` and was refused.
    pub(crate) override_refused: bool,
}

/// Reconcile the setting with the environment. Pure over both, so the whole
/// matrix is a table test with no process environment involved.
///
/// Precedence is `Never` > environment > setting. An unparsable environment
/// value is not an override at all: the setting stands, unrefused.
pub(crate) fn decide(mode: AttachMode, env: Option<&str>) -> Verdict {
    let asked = env.and_then(env_flag);
    match (mode, asked) {
        (AttachMode::Never, asked) => Verdict {
            publish: false,
            override_refused: asked == Some(true),
        },
        (_, Some(asked)) => Verdict {
            publish: asked,
            override_refused: false,
        },
        (mode, None) => Verdict {
            publish: mode == AttachMode::On,
            override_refused: false,
        },
    }
}

/// An [`ATTACH_ENV`] value as a boolean, or `None` when it spells neither.
/// Generous about spelling — the variable is typed by hand — and case-blind.
fn env_flag(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// A published endpoint: everything a client needs to speak to the live bridge.
/// The same two fields a spawned session receives, reached the other way round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Published {
    /// The loopback url the MCP server mounts on.
    pub(crate) url: String,
    /// A bearer token the server minted and has not revoked.
    pub(crate) token: String,
}

/// `~/.termherd/bridge.json` — where a running instance publishes itself.
/// `None` when there is no home directory, in which case nothing is published.
pub(crate) fn bridge_path() -> Option<PathBuf> {
    crate::paths::termherd_dir().map(|dir| dir.join(BRIDGE_FILE))
}

/// Write the published endpoint, readable only by the current user. Overwrites
/// whatever was there — a file left behind by a crash is not a second instance,
/// since the single-instance lock is taken before this runs.
pub(crate) fn publish(path: &Path, published: &Published) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Remove first, then create: the mode is only applied to a file this call
    // creates, so writing over one an earlier version (or a laxer umask) left
    // behind would keep that file's wider permissions.
    withdraw(path)?;
    let json = serde_json::to_string(published)?;
    termherd_pty::write_private(path, &json)
}

/// Remove the published file at teardown. A missing file is success: teardown
/// must not fail because there was nothing to withdraw.
pub(crate) fn withdraw(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Read back a published endpoint. `Err` when the file is absent or does not
/// parse — both mean "no instance is offering the bridge here".
pub(crate) fn read(path: &Path) -> Result<Published, AttachError> {
    let raw = std::fs::read_to_string(path).map_err(|error| {
        // Absent and unreadable are different diagnoses: "no instance is
        // publishing" tells you to switch the setting on, which a file you lack
        // permission to read would never resolve.
        if error.kind() == std::io::ErrorKind::NotFound {
            AttachError::NotPublished
        } else {
            AttachError::Unreadable
        }
    })?;
    serde_json::from_str(&raw).map_err(|_| AttachError::Unreadable)
}

/// The `mcpServers` snippet a client registers, as one line of JSON.
///
/// The *same* text the spawned-session path writes, because it is the same
/// call: one contract, one producer, two callers.
pub(crate) fn mcp_servers(published: &Published) -> String {
    termherd_claude::mcp::servers_config(&published.url, &published.token)
}

/// Why `termherd mcp-config` could not answer with a config.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AttachError {
    /// No published file: no instance is running, or it did not opt in.
    NotPublished,
    /// A published file that exists and cannot be used: unreadable (a mode or an
    /// ownership the current user does not have) or not the JSON expected.
    Unreadable,
    /// The published url refused the connection: the instance that wrote this
    /// file is gone, and the file is stale.
    Stale,
    /// The endpoint answered, but refused the published token. Not the same
    /// failure as a stale file, and must not print a config.
    Unauthorized,
}

/// How long the probe waits on a loopback endpoint before calling it gone.
/// Generous for a local socket, short enough that a launcher script does not
/// appear to hang.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Prove the published endpoint is live by doing the thing the caller actually
/// intends: an MCP `initialize` handshake with the published token.
///
/// A pid or a lock file would only be a proxy for liveness, and a probe that can
/// succeed without exercising the endpoint is not a probe. The handshake fails
/// exactly when the config would have been useless.
pub(crate) fn probe(published: &Published) -> Result<(), AttachError> {
    let (address, path) = split_url(&published.url).ok_or(AttachError::Unreadable)?;
    // Spoken as raw HTTP/1.1 on a blocking socket rather than through a client
    // crate: this runs on the CLI path, which has no tokio runtime and must not
    // grow one just to say hello to a loopback port.
    let mut stream = std::net::TcpStream::connect_timeout(&address, PROBE_TIMEOUT)
        .map_err(|_| AttachError::Stale)?;
    stream
        .set_read_timeout(Some(PROBE_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(PROBE_TIMEOUT)))
        .map_err(|_| AttachError::Stale)?;
    status_of(handshake(&mut stream, &address, &path, &published.token)?)
}

/// The `(address, path)` an `http://host:port/path` url names, or `None` when it
/// is not one this probe can dial. The endpoint is always loopback with a
/// literal port, so a socket address parse is the whole of url handling here.
fn split_url(url: &str) -> Option<(std::net::SocketAddr, String)> {
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    Some((authority.parse().ok()?, format!("/{path}")))
}

/// Send an MCP `initialize` and return the response's status line.
///
/// The handshake is the operation the caller actually intends, which is the
/// point: a cheaper probe — a pid, a connect that never speaks — can succeed
/// against an endpoint that would then refuse every real call.
fn handshake(
    stream: &mut std::net::TcpStream,
    address: &std::net::SocketAddr,
    path: &str,
    token: &str,
) -> Result<String, AttachError> {
    use std::io::Write;

    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"termherd-mcp-config","version":"0"}}}"#;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| AttachError::Stale)?;
    read_status_line(stream)
}

/// Read only as far as the response's first line.
///
/// Reading to end of stream would work exactly as long as the peer honours the
/// `Connection: close` we asked for. A peer that keeps the socket open —
/// another server on a recycled port, a proxy, an SSE stream — would answer
/// perfectly well and still be reported as gone once the read timeout fired.
/// The status line is all this probe reads, so it stops there.
fn read_status_line(stream: &mut std::net::TcpStream) -> Result<String, AttachError> {
    use std::io::Read;

    /// Bound on a status line, so a peer that answers without ever sending a
    /// newline cannot make this read grow without limit.
    const MAX_STATUS_LINE: usize = 8 * 1024;

    let mut raw = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        if let Some(line) = first_line(&raw) {
            return Ok(line);
        }
        if raw.len() >= MAX_STATUS_LINE {
            return Err(AttachError::Stale);
        }
        match stream.read(&mut chunk) {
            // End of stream with no complete line: nothing usable was said.
            Ok(0) => return first_line(&raw).ok_or(AttachError::Stale),
            Ok(read) => raw.extend_from_slice(&chunk[..read]),
            Err(_) => return Err(AttachError::Stale),
        }
    }
}

/// The first CRLF/LF-terminated line in `raw`, or `None` while none is complete.
fn first_line(raw: &[u8]) -> Option<String> {
    let end = raw.iter().position(|byte| *byte == b'\n')?;
    Some(String::from_utf8_lossy(&raw[..end]).trim_end().to_owned())
}

/// Read the verdict out of an HTTP status line. A refused token is not a dead
/// instance, and the two send the caller to opposite places.
fn status_of(status_line: String) -> Result<(), AttachError> {
    if status_line.contains(" 200") {
        return Ok(());
    }
    if status_line.contains(" 401") {
        return Err(AttachError::Unauthorized);
    }
    tracing::warn!(%status_line, "the published bridge answered unusably");
    Err(AttachError::Stale)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole precedence rule as one table: three settings × four
    /// environments. Derived from the two enums' full ranges rather than a
    /// hand-picked sample, so a new mode cannot slip past it silently.
    #[test]
    fn the_environment_overrides_off_and_on_but_never_never() {
        let cases = [
            // (mode, env, publish, override_refused)
            (AttachMode::Off, None, false, false),
            (AttachMode::Off, Some("1"), true, false),
            (AttachMode::Off, Some("0"), false, false),
            (AttachMode::Off, Some("banana"), false, false),
            (AttachMode::On, None, true, false),
            (AttachMode::On, Some("1"), true, false),
            (AttachMode::On, Some("0"), false, false),
            (AttachMode::On, Some("banana"), true, false),
            (AttachMode::Never, None, false, false),
            (AttachMode::Never, Some("1"), false, true),
            (AttachMode::Never, Some("0"), false, false),
            (AttachMode::Never, Some("banana"), false, false),
        ];
        for (mode, env, publish, refused) in cases {
            assert_eq!(
                decide(mode, env),
                Verdict {
                    publish,
                    override_refused: refused
                },
                "{mode:?} with {env:?}"
            );
        }
    }

    /// A `never` that silently ignored an override would leave the user with a
    /// variable that appears to work and does not. The refusal is reported so
    /// the caller can say so.
    #[test]
    fn only_an_override_that_contradicts_never_is_reported_as_refused() {
        assert!(
            decide(AttachMode::Never, Some("true")).override_refused,
            "an override asking to publish against never is refused, and says so"
        );
        assert!(
            !decide(AttachMode::Never, Some("false")).override_refused,
            "an override agreeing with never contradicts nothing"
        );
        assert!(
            !decide(AttachMode::Off, Some("true")).override_refused,
            "off yields to an override rather than refusing it"
        );
    }

    #[test]
    fn env_flag_reads_the_spellings_a_hand_typed_variable_takes() {
        for on in ["1", "true", "TRUE", "yes", "on"] {
            assert_eq!(env_flag(on), Some(true), "{on} means publish");
        }
        for off in ["0", "false", "FALSE", "no", "off"] {
            assert_eq!(env_flag(off), Some(false), "{off} means do not publish");
        }
        for neither in ["", "banana", "2"] {
            assert_eq!(
                env_flag(neither),
                None,
                "{neither:?} is not an override at all"
            );
        }
    }

    fn sample() -> Published {
        Published {
            url: "http://127.0.0.1:52431/mcp".into(),
            token: "0123456789abcdef".into(),
        }
    }

    #[test]
    fn a_published_endpoint_round_trips_through_the_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(BRIDGE_FILE);
        publish(&path, &sample()).expect("publish");
        assert_eq!(read(&path).expect("read back"), sample());
    }

    #[test]
    fn publishing_over_a_file_left_by_a_crash_replaces_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(BRIDGE_FILE);
        publish(
            &path,
            &Published {
                url: "http://127.0.0.1:1/mcp".into(),
                token: "stale".into(),
            },
        )
        .expect("publish the stale endpoint");
        publish(&path, &sample()).expect("republish");
        assert_eq!(
            read(&path).expect("read back"),
            sample(),
            "the new instance's endpoint is the only one on disk"
        );
    }

    /// Tolerating a *missing* file is not tolerating every failure: a teardown
    /// that swallowed a real removal error would leave a live-looking endpoint
    /// on disk and report success. A directory stands in for the unremovable
    /// file, being the one such case a test can create portably.
    #[test]
    fn withdraw_surfaces_a_failure_that_is_not_a_missing_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let occupied = dir.path().join("occupied");
        std::fs::create_dir(&occupied).expect("create a directory in the way");
        assert!(
            withdraw(&occupied).is_err(),
            "a removal that genuinely failed is reported"
        );
    }

    /// The docs name this path, and a client that reads the file by hand
    /// depends on it. A silent move would break both.
    #[test]
    fn the_bridge_file_sits_in_the_app_data_dir() {
        let path = bridge_path().expect("a home directory in the test environment");
        assert!(path.is_absolute(), "got {}", path.display());
        assert!(
            path.ends_with(std::path::Path::new(".termherd").join(BRIDGE_FILE)),
            "got {}",
            path.display()
        );
    }

    #[test]
    fn withdraw_removes_the_file_and_tolerates_its_absence() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(BRIDGE_FILE);
        publish(&path, &sample()).expect("publish");
        withdraw(&path).expect("withdraw");
        assert!(!path.exists(), "the endpoint is no longer offered");
        withdraw(&path).expect("withdrawing nothing is not a failure");
    }

    #[test]
    fn an_absent_or_corrupt_file_reads_as_no_endpoint() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join(BRIDGE_FILE);
        assert_eq!(read(&missing), Err(AttachError::NotPublished));

        let corrupt = dir.path().join("corrupt.json");
        std::fs::write(&corrupt, "{ not json").expect("write");
        assert_eq!(read(&corrupt), Err(AttachError::Unreadable));
    }

    /// The snippet is the one an in-app session already receives. Asserting the
    /// exact bytes is what keeps the two producers from drifting into two
    /// dialects of the same config.
    #[test]
    fn the_snippet_is_the_config_a_spawned_session_receives() {
        assert_eq!(
            mcp_servers(&sample()),
            r#"{"mcpServers":{"termherd":{"type":"http","url":"http://127.0.0.1:52431/mcp","headers":{"Authorization":"Bearer 0123456789abcdef"}}}}"#
        );
    }

    /// "No instance is publishing" tells you to switch the setting on. A file
    /// that exists and cannot be read would never be fixed by that advice, so
    /// the two must not collapse into one diagnosis. A directory stands in for
    /// the unreadable file, being the one such case a test can create portably.
    #[test]
    fn a_file_that_exists_and_cannot_be_read_is_not_reported_as_absent() {
        let dir = tempfile::tempdir().expect("temp dir");
        let occupied = dir.path().join("occupied");
        std::fs::create_dir(&occupied).expect("create a directory in the way");
        assert_eq!(read(&occupied), Err(AttachError::Unreadable));
    }

    /// A peer that answers and keeps the socket open — another server on a
    /// recycled port, a proxy, an SSE stream — is live. Reading to end of stream
    /// would sit on it until the timeout and then report it gone.
    #[test]
    fn a_peer_that_never_closes_is_still_read() {
        use std::io::Write;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("addr");
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n")
                .expect("write a status line");
            socket.flush().expect("flush");
            // Hold the connection open, as a keep-alive peer would.
            std::thread::sleep(std::time::Duration::from_secs(3));
        });

        let mut stream = std::net::TcpStream::connect(address).expect("connect");
        stream
            .set_read_timeout(Some(PROBE_TIMEOUT))
            .expect("read timeout");
        let started = std::time::Instant::now();
        let line = read_status_line(&mut stream);

        assert_eq!(line.as_deref(), Ok("HTTP/1.1 200 OK"));
        assert!(
            started.elapsed() < PROBE_TIMEOUT,
            "the status line is answered without waiting out the timeout, took {:?}",
            started.elapsed()
        );
        drop(stream);
        let _ = server.join();
    }

    /// The other half of "never closes": a peer that also never sends a
    /// newline. Without the bound, the read grows for as long as that peer
    /// keeps talking — so the bound is reachable, and reachable guards get a
    /// test rather than a comment.
    #[test]
    fn a_peer_that_answers_without_a_newline_is_bounded() {
        use std::io::Write;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("addr");
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            // Twice the bound, and not one newline in it.
            let babble = vec![b'x'; 16 * 1024];
            let _ = socket.write_all(&babble);
            let _ = socket.flush();
            std::thread::sleep(std::time::Duration::from_millis(500));
        });

        let mut stream = std::net::TcpStream::connect(address).expect("connect");
        stream
            .set_read_timeout(Some(PROBE_TIMEOUT))
            .expect("read timeout");
        assert_eq!(
            read_status_line(&mut stream),
            Err(AttachError::Stale),
            "an answer with no line in it is not an answer"
        );
        drop(stream);
        let _ = server.join();
    }

    #[test]
    fn first_line_waits_for_a_complete_line() {
        assert_eq!(first_line(b"HTTP/1.1 200 OK"), None, "no newline yet");
        assert_eq!(
            first_line(b"HTTP/1.1 200 OK\r\nContent-Type: x\r\n").as_deref(),
            Some("HTTP/1.1 200 OK"),
            "the line's own terminator is not part of it"
        );
    }

    /// A url nothing is listening on is the crash case: the file outlived the
    /// instance that wrote it.
    #[test]
    fn a_closed_port_reads_as_stale() {
        // Bind and drop, so the port is one nothing answers on any more.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        drop(listener);
        assert_eq!(
            probe(&Published {
                url: format!("http://{addr}/mcp"),
                token: "whatever".into(),
            }),
            Err(AttachError::Stale)
        );
    }

    #[tokio::test]
    async fn a_live_endpoint_with_its_published_token_probes_clean() {
        let (bridge, _requests) = crate::shell::bridge::channel();
        let tokens = crate::mcp::Tokens::default();
        let endpoint = crate::mcp::serve(bridge, tokens.clone())
            .await
            .expect("bind the loopback server");
        let published = Published {
            url: endpoint.url.clone(),
            token: tokens.issue(),
        };
        let probed = tokio::task::spawn_blocking(move || probe(&published))
            .await
            .expect("join");
        assert_eq!(probed, Ok(()));
    }

    /// A revoked or forged token is a different failure from a dead port, and
    /// collapsing the two would send the caller looking for a process that is
    /// running perfectly well.
    #[tokio::test]
    async fn a_live_endpoint_refusing_the_token_is_unauthorized_not_stale() {
        let (bridge, _requests) = crate::shell::bridge::channel();
        let endpoint = crate::mcp::serve(bridge, crate::mcp::Tokens::default())
            .await
            .expect("bind the loopback server");
        let published = Published {
            url: endpoint.url.clone(),
            token: "forged".into(),
        };
        let probed = tokio::task::spawn_blocking(move || probe(&published))
            .await
            .expect("join");
        assert_eq!(probed, Err(AttachError::Unauthorized));
    }
}
