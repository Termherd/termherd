//! `termherd mcp-proxy` — stdio-to-HTTP bridge for agent CLIs without a
//! `--mcp-config` flag (such as Antigravity).
//!
//! When TermHerd spawns an agent session it writes the session's private `0600`
//! `mcpServers` JSON file and exports its path in `TERMHERD_MCP_CONFIG`. A CLI
//! registered once with `termherd mcp-proxy` inherits that variable, so every
//! session launched from TermHerd gets all live workspace tools over a
//! per-session revocable bearer token.
//!
//! No silent fallback:
//! - When `TERMHERD_MCP_CONFIG` is **set** but unreadable, malformed, or the
//!   live bridge refuses/drops the request, the proxy reports an error (exit 1
//!   at startup, or a JSON-RPC error per request) and never falls back to
//!   `settings.json`.
//! - When `TERMHERD_MCP_CONFIG` is **absent** (launched outside TermHerd, or a
//!   host CLI that does not propagate the variable), the proxy logs why on
//!   stderr and serves the `settings.json` tools with explicit `initialize`
//!   `instructions` stating why the live tools are unavailable.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use thiserror::Error;

/// Timeout for establishing the loopback TCP connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Timeout for writing a request to the loopback bridge.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// Timeout for reading a reply from the loopback bridge. Must exceed the live
/// bridge's longest bounded tool call (`wait_for_status` caps at 60 s).
const READ_TIMEOUT: Duration = Duration::from_secs(120);
/// Upper bound on HTTP response headers read from the loopback bridge.
const MAX_HEADER_BYTES: usize = 64 * 1024;
/// Upper bound on an HTTP response body (screenshots are base64 PNGs).
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Server instructions emitted in `initialize` when `TERMHERD_MCP_CONFIG` is
/// absent, so neither the user nor the model mistakes settings-only mode for a
/// working live bridge.
pub(crate) const MISSING_ENV_INSTRUCTIONS: &str = "TermHerd live bridge is not active because TERMHERD_MCP_CONFIG is not set in this \
     process's environment (launched outside TermHerd, or the host CLI did not propagate \
     the variable). Only settings.json tools (list_options, set_option) are available.";

/// Parsed loopback target extracted from a `termherd-mcp-<id>.json` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BridgeTarget {
    pub(crate) addr: SocketAddr,
    pub(crate) host_header: String,
    pub(crate) path: String,
    pub(crate) token: String,
}

#[derive(Debug, Error)]
pub(crate) enum ProxyConfigError {
    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("config is not valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("config is missing mcpServers.termherd")]
    MissingServer,
    #[error("bridge URL is not a loopback HTTP endpoint: {0}")]
    InvalidUrl(String),
    #[error("config is missing a Bearer Authorization header")]
    MissingToken,
}

#[derive(Debug, Error)]
pub(crate) enum TransportError {
    #[error("could not connect to live bridge at {addr}: {source}")]
    Connect {
        addr: SocketAddr,
        #[source]
        source: io::Error,
    },
    #[error("I/O error talking to live bridge: {0}")]
    Io(#[from] io::Error),
    #[error("malformed HTTP response from live bridge: {0}")]
    MalformedHttp(String),
    #[error("live bridge refused the session token (HTTP {0})")]
    Unauthorized(u16),
    #[error("live bridge returned HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HttpOutcome {
    Json(String),
    NoContent,
}

/// Entry point for `termherd mcp-proxy`.
///
/// `config_path` comes from `TERMHERD_MCP_CONFIG` read once in `main()`.
/// Returns `Ok(())` on clean EOF, or `Err(diagnostic)` when a configured bridge
/// file cannot be loaded.
pub(crate) fn run(config_path: Option<&Path>, settings_path: Option<&Path>) -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    match config_path {
        Some(path) => {
            let target = read_config(path).map_err(|err| err.to_string())?;
            serve_bridge(stdin.lock(), stdout.lock(), &target);
            Ok(())
        }
        None => {
            let _ = writeln!(
                io::stderr(),
                "termherd mcp-proxy: {} is not set; live workspace tools are unavailable \
                 (serving settings.json tools only)",
                termherd_core::locations::MCP_CONFIG_VAR
            );
            serve_settings_only(stdin.lock(), stdout.lock(), settings_path);
            Ok(())
        }
    }
}

/// Parse the `mcpServers.termherd` entry written by
/// [`termherd_claude::mcp::servers_config`].
pub(crate) fn read_config(path: &Path) -> Result<BridgeTarget, ProxyConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ProxyConfigError::Io {
        path: path.display().to_string(),
        source,
    })?;
    parse_config(&text)
}

pub(crate) fn parse_config(text: &str) -> Result<BridgeTarget, ProxyConfigError> {
    let value: Value = serde_json::from_str(text)?;
    let server = value
        .get("mcpServers")
        .and_then(|servers| servers.get("termherd"))
        .ok_or(ProxyConfigError::MissingServer)?;
    let url = server
        .get("url")
        .and_then(Value::as_str)
        .ok_or(ProxyConfigError::MissingServer)?;
    let (addr, host_header, path) = parse_loopback_url(url)?;
    let auth = server
        .get("headers")
        .and_then(|headers| headers.get("Authorization"))
        .and_then(Value::as_str)
        .ok_or(ProxyConfigError::MissingToken)?;
    let token = auth
        .strip_prefix("Bearer ")
        .filter(|t| !t.is_empty() && !t.contains('\r') && !t.contains('\n'))
        .ok_or(ProxyConfigError::MissingToken)?;
    Ok(BridgeTarget {
        addr,
        host_header,
        path,
        token: token.to_owned(),
    })
}

fn parse_loopback_url(url: &str) -> Result<(SocketAddr, String, String), ProxyConfigError> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| ProxyConfigError::InvalidUrl(url.to_owned()))?;
    let (authority, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    if path.contains('\r') || path.contains('\n') || path.contains(' ') {
        return Err(ProxyConfigError::InvalidUrl(url.to_owned()));
    }
    let (host, port_str) = authority
        .rsplit_once(':')
        .ok_or_else(|| ProxyConfigError::InvalidUrl(url.to_owned()))?;
    let port: u16 = port_str
        .parse()
        .map_err(|_| ProxyConfigError::InvalidUrl(url.to_owned()))?;
    let ip = match host {
        "127.0.0.1" | "localhost" => IpAddr::V4(Ipv4Addr::LOCALHOST),
        "[::1]" => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        _ => return Err(ProxyConfigError::InvalidUrl(url.to_owned())),
    };
    Ok((
        SocketAddr::new(ip, port),
        format!("{host}:{port}"),
        path.to_owned(),
    ))
}

/// Forward newline-delimited JSON-RPC messages on `reader` to `target` and
/// write replies to `writer`.
pub(crate) fn serve_bridge<R: BufRead, W: Write>(reader: R, mut writer: W, target: &BridgeTarget) {
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let reply = match post_json(target, trimmed) {
            Ok(HttpOutcome::Json(body)) => Some(compact_single_line(&body)),
            Ok(HttpOutcome::NoContent) => None,
            Err(err) => {
                let _ = writeln!(io::stderr(), "termherd mcp-proxy: {err}");
                jsonrpc_error_response(&message, &err.to_string()).map(|v| v.to_string())
            }
        };
        if let Some(reply) = reply {
            if writeln!(writer, "{reply}").is_err() {
                break;
            }
            if writer.flush().is_err() {
                break;
            }
        }
    }
}

/// Serve the `settings.json` surface when `TERMHERD_MCP_CONFIG` is absent,
/// injecting [`MISSING_ENV_INSTRUCTIONS`] into `initialize` so the missing
/// live bridge is never silent.
pub(crate) fn serve_settings_only<R: BufRead, W: Write>(
    reader: R,
    mut writer: W,
    settings_path: Option<&Path>,
) {
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let (settings, writable) = match settings_path {
            Some(path) => match termherd_mcp::file::read_object(path) {
                Ok(value) => (value, Ok(())),
                Err(reason) => (json!({}), Err(reason)),
            },
            None => (json!({}), Ok(())),
        };
        let reply = termherd_mcp::handle_message(&message, &settings);
        let mut response = reply.response;
        if message.get("method").and_then(Value::as_str) == Some("initialize")
            && let Some(Value::Object(ref mut obj)) = response
            && let Some(Value::Object(result)) = obj.get_mut("result")
        {
            result.insert(
                "instructions".to_owned(),
                Value::String(MISSING_ENV_INSTRUCTIONS.to_owned()),
            );
        }
        if let Some(new_settings) = reply.write_settings
            && let Err(err) = writable.and_then(|()| {
                let path = settings_path.ok_or("no home directory; cannot write settings")?;
                termherd_mcp::file::write(path, &new_settings)
            })
        {
            let _ = writeln!(io::stderr(), "termherd mcp-proxy: {err}");
            response =
                jsonrpc_error_response(&message, &format!("could not persist settings: {err}"));
        }
        if let Some(response) = response {
            if writeln!(writer, "{response}").is_err() {
                break;
            }
            if writer.flush().is_err() {
                break;
            }
        }
    }
}

fn post_json(target: &BridgeTarget, body: &str) -> Result<HttpOutcome, TransportError> {
    let mut stream =
        TcpStream::connect_timeout(&target.addr, CONNECT_TIMEOUT).map_err(|source| {
            TransportError::Connect {
                addr: target.addr,
                source,
            }
        })?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;

    let request = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {host}\r\n\
         Authorization: Bearer {token}\r\n\
         Content-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        path = target.path,
        host = target.host_header,
        token = target.token,
        len = body.len(),
    );
    stream.write_all(request.as_bytes())?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let (status, content_length, chunked) = read_response_head(&mut reader)?;
    let raw_body = read_response_body(&mut reader, content_length, chunked)?;
    let text = String::from_utf8(raw_body)
        .map_err(|err| TransportError::MalformedHttp(err.to_string()))?;

    match status {
        200..=299 => {
            if text.trim().is_empty() {
                Ok(HttpOutcome::NoContent)
            } else {
                Ok(HttpOutcome::Json(text))
            }
        }
        401 | 403 => Err(TransportError::Unauthorized(status)),
        _ => Err(TransportError::HttpStatus { status, body: text }),
    }
}

fn read_response_head<R: BufRead>(
    reader: &mut R,
) -> Result<(u16, Option<usize>, bool), TransportError> {
    let mut status_line = String::new();
    let n = reader.read_line(&mut status_line)?;
    if n == 0 {
        return Err(TransportError::MalformedHttp(
            "empty response from live bridge".to_owned(),
        ));
    }
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| TransportError::MalformedHttp(status_line.trim().to_owned()))?;

    let mut header_bytes = n;
    let mut content_length = None;
    let mut chunked = false;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            break;
        }
        header_bytes += read;
        if header_bytes > MAX_HEADER_BYTES {
            return Err(TransportError::MalformedHttp(
                "response headers exceeded limit".to_owned(),
            ));
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().ok();
            } else if name.eq_ignore_ascii_case("transfer-encoding")
                && value.to_ascii_lowercase().contains("chunked")
            {
                chunked = true;
            }
        }
    }
    Ok((status, content_length, chunked))
}

fn read_response_body<R: BufRead>(
    reader: &mut R,
    content_length: Option<usize>,
    chunked: bool,
) -> Result<Vec<u8>, TransportError> {
    if chunked {
        return read_chunked_body(reader);
    }
    if let Some(len) = content_length {
        if len > MAX_BODY_BYTES {
            return Err(TransportError::MalformedHttp(
                "response body exceeded limit".to_owned(),
            ));
        }
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf)?;
        return Ok(buf);
    }
    let mut buf = Vec::new();
    reader
        .take((MAX_BODY_BYTES + 1) as u64)
        .read_to_end(&mut buf)?;
    if buf.len() > MAX_BODY_BYTES {
        return Err(TransportError::MalformedHttp(
            "response body exceeded limit".to_owned(),
        ));
    }
    Ok(buf)
}

fn read_chunked_body<R: BufRead>(reader: &mut R) -> Result<Vec<u8>, TransportError> {
    let mut out = Vec::new();
    loop {
        let mut size_line = String::new();
        if reader.read_line(&mut size_line)? == 0 {
            return Err(TransportError::MalformedHttp(
                "unexpected EOF in chunked body".to_owned(),
            ));
        }
        let hex = size_line.trim().split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(hex, 16)
            .map_err(|_| TransportError::MalformedHttp(format!("bad chunk size: {hex}")))?;
        if size == 0 {
            break;
        }
        if out.len().saturating_add(size) > MAX_BODY_BYTES {
            return Err(TransportError::MalformedHttp(
                "chunked response body exceeded limit".to_owned(),
            ));
        }
        let start = out.len();
        out.resize(start + size, 0);
        reader.read_exact(&mut out[start..])?;
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf)?;
    }
    Ok(out)
}

fn compact_single_line(body: &str) -> String {
    match serde_json::from_str::<Value>(body) {
        Ok(value) => value.to_string(),
        Err(_) => body.trim().replace(['\r', '\n'], " "),
    }
}

fn jsonrpc_error_response(message: &Value, detail: &str) -> Option<Value> {
    let id = message.get("id")?;
    Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32000,
            "message": format!("termherd live bridge error: {detail}"),
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_config_roundtrips_servers_config() {
        let raw = termherd_claude::mcp::servers_config("http://127.0.0.1:52431/mcp", "secret123");
        let target = parse_config(&raw).expect("valid config");
        assert_eq!(target.addr, SocketAddr::from(([127, 0, 0, 1], 52431)));
        assert_eq!(target.host_header, "127.0.0.1:52431");
        assert_eq!(target.path, "/mcp");
        assert_eq!(target.token, "secret123");
    }

    #[test]
    fn parse_config_rejects_non_loopback_url_and_missing_token() {
        let remote = termherd_claude::mcp::servers_config("http://192.168.1.5:52431/mcp", "tok");
        assert!(matches!(
            parse_config(&remote),
            Err(ProxyConfigError::InvalidUrl(_))
        ));

        let no_token =
            r#"{"mcpServers":{"termherd":{"type":"http","url":"http://127.0.0.1:52431/mcp"}}}"#;
        assert!(matches!(
            parse_config(no_token),
            Err(ProxyConfigError::MissingToken)
        ));
    }

    #[test]
    fn read_chunked_body_reassembles_chunks() {
        let wire = b"5\r\n{\"a\":\r\n2\r\n1}\r\n0\r\n\r\n";
        let mut cursor = io::Cursor::new(&wire[..]);
        let decoded = read_chunked_body(&mut cursor).expect("chunked body");
        assert_eq!(decoded, br#"{"a":1}"#);
    }

    #[test]
    fn settings_only_initialize_explains_why_live_tools_are_missing() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
            "\n"
        );
        let mut output = Vec::new();
        serve_settings_only(io::Cursor::new(input.as_bytes()), &mut output, None);
        let line = String::from_utf8(output).expect("utf8");
        let resp: Value = serde_json::from_str(line.trim()).expect("json response");
        let instructions = resp["result"]["instructions"]
            .as_str()
            .expect("explicit instructions");
        assert!(
            instructions.contains("TERMHERD_MCP_CONFIG"),
            "instructions must name the missing env var, got: {instructions}"
        );
    }

    #[tokio::test]
    async fn proxy_forwards_initialize_and_tools_list_to_real_live_bridge() {
        let (bridge, _requests) = crate::shell::bridge::channel();
        let tokens = crate::mcp::Tokens::default();
        let endpoint = crate::mcp::serve(bridge, tokens.clone(), None)
            .await
            .expect("bind loopback server");
        let token = tokens.issue();
        let config_json = termherd_claude::mcp::servers_config(&endpoint.url, &token);
        let target = parse_config(&config_json).expect("parse config");

        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
            "\n"
        );

        let output = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            serve_bridge(io::Cursor::new(input.as_bytes()), &mut out, &target);
            String::from_utf8(out).expect("utf8")
        })
        .await
        .expect("join");

        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(
            lines.len(),
            2,
            "initialize and tools/list reply; notification emits nothing: {output}"
        );
        let init: Value = serde_json::from_str(lines[0]).expect("init json");
        assert_eq!(init["id"], 1);
        assert!(init["result"]["serverInfo"].is_object());

        let list: Value = serde_json::from_str(lines[1]).expect("tools/list json");
        assert_eq!(list["id"], 2);
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(names.contains(&"snapshot"), "missing snapshot: {names:?}");
        assert!(
            names.contains(&"list_sessions"),
            "missing list_sessions: {names:?}"
        );
        assert!(
            names.contains(&"read_terminal"),
            "missing read_terminal: {names:?}"
        );
    }

    #[tokio::test]
    async fn proxy_returns_jsonrpc_error_when_token_is_revoked() {
        let (bridge, _requests) = crate::shell::bridge::channel();
        let tokens = crate::mcp::Tokens::default();
        let endpoint = crate::mcp::serve(bridge, tokens.clone(), None)
            .await
            .expect("bind loopback server");
        let token = tokens.issue();
        tokens.revoke(&token);
        let config_json = termherd_claude::mcp::servers_config(&endpoint.url, &token);
        let target = parse_config(&config_json).expect("parse config");

        let input = concat!(
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/list","params":{}}"#,
            "\n"
        );
        let output = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            serve_bridge(io::Cursor::new(input.as_bytes()), &mut out, &target);
            String::from_utf8(out).expect("utf8")
        })
        .await
        .expect("join");

        let resp: Value = serde_json::from_str(output.trim()).expect("json error");
        assert_eq!(resp["id"], 7);
        assert!(
            resp["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("401")),
            "expected 401 in error message, got: {resp}"
        );
    }
}
