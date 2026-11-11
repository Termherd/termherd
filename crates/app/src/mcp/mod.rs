//! In-process MCP server — the live-bridge gate.
//!
//! termherd hosts its own Model Context Protocol server on **loopback**, so the
//! Claude sessions it launches can reach back into the running app. The server
//! runs on the async-transport runtime and answers tool calls by talking to
//! `core::App` over the timeout-bounded bridge — it never touches core directly.
//!
//! rmcp carries the wire protocol (JSON-RPC, the initialize handshake, tool
//! listing/calls); this module owns what rmcp leaves to the host: binding the
//! loopback listener and gating every request. The gate first refuses (403) a
//! request whose `Host` or `Origin` names anything but this listener — a
//! loopback literal on the bound port — which defeats DNS rebinding without
//! relying on rmcp's defaults. It then requires a **per-session bearer token**
//! (401), the per-session authorization on top.

mod handler;

use std::collections::HashSet;
use std::convert::Infallible;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http::header::{AUTHORIZATION, HOST, ORIGIN};
use http::{HeaderMap, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::net::TcpListener;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::shell::bridge::BridgeHandle;
use handler::TermherdMcp;

/// The loopback path the server mounts on; an `mcpServers` url points here.
const MCP_PATH: &str = "/mcp";

/// The per-session bearer tokens the loopback server accepts. Shared (cloned
/// `Arc`) between the server — which validates every request — and the launch
/// path, which mints one per Claude session and revokes it on close. A token is
/// a v4 UUID from the OS CSPRNG and is never logged.
#[derive(Clone, Default)]
pub(crate) struct Tokens(Arc<Mutex<HashSet<String>>>);

impl Tokens {
    /// Mint, register, and return a fresh token.
    pub(crate) fn issue(&self) -> String {
        let token = Uuid::new_v4().simple().to_string();
        if let Ok(mut set) = self.0.lock() {
            set.insert(token.clone());
        }
        token
    }

    /// Revoke a token once its session closes. A no-op for an unknown token.
    pub(crate) fn revoke(&self, token: &str) {
        if let Ok(mut set) = self.0.lock() {
            set.remove(token);
        }
    }

    /// Whether `token` is one this server minted and has not revoked. A poisoned
    /// lock fails closed (rejects), never panics.
    fn accepts(&self, token: &str) -> bool {
        self.0
            .lock()
            .map(|set| set.contains(token))
            .unwrap_or(false)
    }
}

/// Where the running MCP server can be reached — the loopback URL an
/// `mcpServers` config points a Claude session at. The token is per session, so
/// it is not part of the endpoint.
#[derive(Clone, Debug)]
pub(crate) struct Endpoint {
    pub(crate) url: String,
}

/// Bind the loopback MCP server and start accepting connections on the current
/// tokio runtime. Returns the [`Endpoint`] once the listener is bound (so the
/// launch path can inject its url); the accept loop then runs until the runtime
/// is dropped. Every request is gated on its `Host`, its `Origin` and one of
/// `tokens` before rmcp sees it.
pub(crate) async fn serve(
    bridge: BridgeHandle,
    tokens: Tokens,
    settings_file: Option<PathBuf>,
) -> std::io::Result<Endpoint> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let addr = listener.local_addr()?;
    let endpoint = Endpoint {
        url: format!("http://{addr}{MCP_PATH}"),
    };
    let service = Arc::new(build_service(bridge, settings_file));
    let gate = Gate {
        tokens,
        port: addr.port(),
    };
    tokio::spawn(accept_loop(listener, service, gate));
    Ok(endpoint)
}

/// The rmcp request→response handler, rebuilt per session by the factory (a
/// fresh [`TermherdMcp`] cloning the bridge). Stateless JSON responses: each
/// tool call is self-contained, so no SSE session bookkeeping.
fn build_service(
    bridge: BridgeHandle,
    settings_file: Option<PathBuf>,
) -> StreamableHttpService<TermherdMcp, LocalSessionManager> {
    // Stateless request/response with a plain JSON body: each call is
    // self-contained, so no SSE session bookkeeping is needed. Many tools
    // mutate the workspace, which is why every request is gated first.
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true);
    StreamableHttpService::new(
        move || Ok(TermherdMcp::new(bridge.clone()).with_settings_file(settings_file.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    )
}

/// What every request must satisfy before rmcp sees it.
#[derive(Clone)]
struct Gate {
    tokens: Tokens,
    /// The loopback port the listener bound, the only one a request may name.
    port: u16,
}

/// Accept loopback connections forever, serving each on its own task. One shared
/// rmcp service backs every connection; the per-request gate wraps it.
async fn accept_loop(
    listener: TcpListener,
    service: Arc<StreamableHttpService<TermherdMcp, LocalSessionManager>>,
    gate: Gate,
) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _peer)) => stream,
            Err(error) => {
                warn!(%error, "mcp server: accept failed");
                continue;
            }
        };
        let io = TokioIo::new(stream);
        let service = Arc::clone(&service);
        let gate = gate.clone();
        tokio::spawn(async move {
            let handler = service_fn(move |req| dispatch(req, Arc::clone(&service), gate.clone()));
            if let Err(error) = http1::Builder::new().serve_connection(io, handler).await {
                debug!(%error, "mcp server: connection closed with error");
            }
        });
    }
}

/// Gate one request, then hand it to rmcp. A request that does not name this
/// listener is a 403, and a missing or unknown token a 401, both before rmcp
/// ever parses the body.
async fn dispatch(
    req: Request<Incoming>,
    service: Arc<StreamableHttpService<TermherdMcp, LocalSessionManager>>,
    gate: Gate,
) -> Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    if !(host_allowed(req.headers(), gate.port) && origin_allowed(req.headers(), gate.port)) {
        return Ok(bare(StatusCode::FORBIDDEN, b"forbidden"));
    }
    if !authorized(req.headers(), &gate.tokens) {
        return Ok(bare(StatusCode::UNAUTHORIZED, b"unauthorized"));
    }
    Ok(service.handle(req).await)
}

/// The host names a request may use to reach the loopback listener.
const LOOPBACK_HOSTS: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];

/// Whether `authority` (`host:port`) names this listener: a loopback literal,
/// in any case, on exactly the bound port. The port is compared as text so
/// `+8080` or `08080` cannot pass as `8080`.
fn is_own_authority(authority: &str, port: u16) -> bool {
    authority.rsplit_once(':').is_some_and(|(host, given)| {
        given == port.to_string()
            && LOOPBACK_HOSTS
                .iter()
                .any(|loopback| host.eq_ignore_ascii_case(loopback))
    })
}

/// Whether the request carries exactly one `Host`, and it names this listener.
/// A page that rebinds its own domain to 127.0.0.1 still sends that domain
/// here, which is what this refuses.
fn host_allowed(headers: &HeaderMap, port: u16) -> bool {
    let mut hosts = headers.get_all(HOST).iter();
    match (hosts.next(), hosts.next()) {
        (Some(host), None) => host.to_str().is_ok_and(|host| is_own_authority(host, port)),
        _ => false,
    }
}

/// Whether every `Origin` is this listener's own `http://` origin. A client
/// that is not a browser sends none, and passes.
fn origin_allowed(headers: &HeaderMap, port: u16) -> bool {
    headers.get_all(ORIGIN).iter().all(|origin| {
        origin
            .to_str()
            .ok()
            .and_then(|origin| origin.strip_prefix("http://"))
            .is_some_and(|authority| is_own_authority(authority, port))
    })
}

/// Whether the headers carry an `Authorization: Bearer <token>` the server
/// minted. Pure over the headers so it is unit-testable without a socket.
fn authorized(headers: &HeaderMap, tokens: &Tokens) -> bool {
    bearer_token(headers).is_some_and(|token| tokens.accepts(token))
}

/// The token from an `Authorization: Bearer <token>` header, if well-formed.
/// The scheme match is case-insensitive per RFC 7235, so a client sending
/// `bearer` (or `BEARER`) is not spuriously rejected.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("Bearer")
        .then(|| token.trim())
        .filter(|token| !token.is_empty())
}

/// A refusal with a fixed body, built without a fallible builder so it never
/// panics and never needs an `unwrap`.
fn bare(status: StatusCode, body: &'static [u8]) -> Response<BoxBody<Bytes, Infallible>> {
    let mut response = Response::new(Full::new(Bytes::from_static(body)).boxed());
    *response.status_mut() = status;
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_map(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(value) = value.parse() {
            headers.insert(AUTHORIZATION, value);
        }
        headers
    }

    #[test]
    fn issue_registers_a_token_that_is_then_accepted() {
        let tokens = Tokens::default();
        let token = tokens.issue();
        assert!(tokens.accepts(&token), "a freshly issued token is accepted");
        assert!(
            !tokens.accepts("not-a-real-token"),
            "an unknown token is rejected"
        );
    }

    #[test]
    fn revoke_stops_accepting_a_token() {
        let tokens = Tokens::default();
        let token = tokens.issue();
        tokens.revoke(&token);
        assert!(!tokens.accepts(&token), "a revoked token is rejected");
    }

    #[test]
    fn each_issued_token_is_distinct() {
        let tokens = Tokens::default();
        assert_ne!(tokens.issue(), tokens.issue(), "tokens are unique");
    }

    #[test]
    fn bearer_token_parses_only_a_well_formed_header() {
        assert_eq!(
            bearer_token(&header_map("Bearer abc123")),
            Some("abc123"),
            "a Bearer header yields its token"
        );
        assert_eq!(
            bearer_token(&header_map("bearer abc123")),
            Some("abc123"),
            "the scheme match is case-insensitive (RFC 7235)"
        );
        assert_eq!(
            bearer_token(&header_map("abc123")),
            None,
            "a bare token without the scheme is not accepted"
        );
        assert_eq!(
            bearer_token(&HeaderMap::new()),
            None,
            "no Authorization header yields no token"
        );
    }

    #[tokio::test]
    async fn server_answers_401_to_a_request_without_a_token() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (bridge, _requests) = crate::shell::bridge::channel();
        let endpoint = serve(bridge, Tokens::default(), None)
            .await
            .expect("bind the loopback server");
        let authority = authority(&endpoint);

        let mut stream = tokio::net::TcpStream::connect(&authority)
            .await
            .expect("connect to the loopback server");
        let request = format!(
            "POST {MCP_PATH} HTTP/1.1\r\nHost: {authority}\r\n\
             Content-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}"
        );
        stream
            .write_all(request.as_bytes())
            .await
            .expect("write the request");
        let mut buf = [0u8; 256];
        let read = stream.read(&mut buf).await.expect("read the response");
        let status_line = String::from_utf8_lossy(&buf[..read]);
        assert!(
            status_line.starts_with("HTTP/1.1 401"),
            "an unauthenticated request is refused before rmcp, got: {}",
            status_line.lines().next().unwrap_or_default()
        );
    }

    /// One raw HTTP request/response round-trip against the loopback server,
    /// returning `(status_line, body)`. Crafts the bytes by hand so the test
    /// needs no MCP client crate.
    async fn round_trip(authority: &str, headers: &str, body: &str) -> (String, String) {
        round_trip_with_host(authority, authority, headers, body).await
    }

    /// [`round_trip`] with the `Host` header set to `host` rather than the
    /// authority actually connected to, as a DNS-rebinding page would send.
    async fn round_trip_with_host(
        authority: &str,
        host: &str,
        headers: &str,
        body: &str,
    ) -> (String, String) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut stream = tokio::net::TcpStream::connect(authority)
            .await
            .expect("connect to the loopback server");
        let request = format!(
            "POST {MCP_PATH} HTTP/1.1\r\nHost: {host}\r\n{headers}\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .await
            .expect("write the request");
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .await
            .expect("read the response");
        let text = String::from_utf8_lossy(&raw).into_owned();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
        let status = head.lines().next().unwrap_or_default().to_owned();
        (status, body.to_owned())
    }

    fn authority(endpoint: &Endpoint) -> String {
        endpoint
            .url
            .strip_prefix("http://")
            .and_then(|rest| rest.split('/').next())
            .expect("an http authority in the endpoint url")
            .to_owned()
    }

    /// The body of an MCP `initialize` request.
    const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}"#;

    #[tokio::test]
    async fn a_valid_token_completes_the_mcp_initialize_handshake() {
        let (bridge, _requests) = crate::shell::bridge::channel();
        let tokens = Tokens::default();
        let endpoint = serve(bridge, tokens.clone(), None).await.expect("bind");
        let token = tokens.issue();

        let (status, body) =
            round_trip(&authority(&endpoint), &call_headers(&token), INITIALIZE).await;

        assert!(
            status.contains("200"),
            "an authenticated initialize succeeds, got status: {status}"
        );
        assert!(
            body.contains("termherd") && body.contains("result"),
            "the handshake returns termherd's serverInfo, got body: {body}"
        );
    }

    /// Headers for an authenticated JSON tool call.
    fn call_headers(token: &str) -> String {
        format!(
            "Authorization: Bearer {token}\r\nContent-Type: application/json\r\n\
             Accept: application/json, text/event-stream\r\n"
        )
    }

    #[tokio::test]
    async fn the_screenshot_tool_is_advertised_alongside_the_text_reads() {
        let (bridge, _requests) = crate::shell::bridge::channel();
        let tokens = Tokens::default();
        let endpoint = serve(bridge, tokens.clone(), None).await.expect("bind");
        let token = tokens.issue();

        let (status, body) = round_trip(
            &authority(&endpoint),
            &call_headers(&token),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .await;

        assert!(status.contains("200"), "got status: {status}");
        assert!(
            body.contains("\"screenshot\""),
            "the pixel companion is listed with the other tools, got: {body}"
        );
    }

    /// The wire proof: an authenticated `tools/call` reaches the handler, the
    /// bridge answers with encoded pixels, and the caller receives them as MCP
    /// image content — the one claim the handler's own tests cannot make,
    /// since they never cross the JSON-RPC boundary.
    #[tokio::test]
    async fn a_screenshot_call_carries_png_bytes_back_as_image_content() {
        use crate::shell::bridge::{Reply, ShotResult, spawn_test_shell};

        let (bridge, requests) = crate::shell::bridge::channel();
        let _shell = spawn_test_shell(
            requests,
            Reply::Shot(ShotResult {
                png: Some(vec![1, 2, 3]),
                width: 640,
                height: 400,
                error: None,
            }),
        );
        let tokens = Tokens::default();
        let endpoint = serve(bridge, tokens.clone(), None).await.expect("bind");
        let token = tokens.issue();

        let (status, body) = round_trip(
            &authority(&endpoint),
            &call_headers(&token),
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"screenshot","arguments":{"max_width":640}}}"#,
        )
        .await;

        assert!(status.contains("200"), "got status: {status}");
        assert!(
            body.contains("image/png") && body.contains("\"AQID\""),
            "the three bytes arrive base64-encoded as image content, got: {body}"
        );
        assert!(
            body.contains("\"height\":400"),
            "the produced size travels alongside the image, got: {body}"
        );
    }

    #[tokio::test]
    async fn the_keyboard_tools_are_advertised_with_the_rest() {
        let (bridge, _requests) = crate::shell::bridge::channel();
        let tokens = Tokens::default();
        let endpoint = serve(bridge, tokens.clone(), None).await.expect("bind");
        let token = tokens.issue();

        let (status, body) = round_trip(
            &authority(&endpoint),
            &call_headers(&token),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .await;

        assert!(status.contains("200"), "got status: {status}");
        for tool in ["press_keys", "run_action"] {
            assert!(
                body.contains(&format!("\"{tool}\"")),
                "{tool} must be listed, got: {body}"
            );
        }
    }

    /// The wire proof for the keyboard rung: an authenticated `tools/call`
    /// reaches the handler, the chord arrives parsed, and the per-press steps
    /// come back as structured content. The handler's own tests never cross the
    /// JSON-RPC boundary, so this is the one place that claim is made.
    #[tokio::test]
    async fn a_press_keys_call_carries_its_steps_back_as_structured_content() {
        use crate::shell::bridge::{PressOutcome, PressStep, Reply, Request, spawn_test_shell};

        let (bridge, requests) = crate::shell::bridge::channel();
        let shell = spawn_test_shell(
            requests,
            Reply::Pressed(PressOutcome {
                steps: vec![
                    PressStep::Ran("close-focused".into()),
                    PressStep::Overlay("tab-close-confirm".into()),
                ],
                focused: Some("7".into()),
                error: None,
            }),
        );
        let tokens = Tokens::default();
        let endpoint = serve(bridge, tokens.clone(), None).await.expect("bind");
        let token = tokens.issue();

        let (status, body) = round_trip(
            &authority(&endpoint),
            &call_headers(&token),
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"press_keys","arguments":{"keys":["cmd+w","cmd+d"]}}}"#,
        )
        .await;

        assert!(status.contains("200"), "got status: {status}");
        assert!(
            body.contains("\"action\":\"close-focused\""),
            "the action that ran must reach the caller, got: {body}"
        );
        assert!(
            body.contains("\"overlay\":\"tab-close-confirm\""),
            "so must the prompt that ate the second chord, got: {body}"
        );
        assert!(
            body.contains("\"focused_handle\":\"7\""),
            "act→observe in one round trip, got: {body}"
        );
        // The chords crossed the boundary parsed, not as raw strings.
        match shell.await.expect("shell task") {
            Request::Press(presses) => assert_eq!(presses.len(), 2, "got: {presses:?}"),
            other => panic!("expected a press request, got {other:?}"),
        }
    }

    const PORT: u16 = 49_152;

    fn headers_with(entries: &[(http::header::HeaderName, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in entries {
            headers.append(name.clone(), value.parse().expect("a valid header value"));
        }
        headers
    }

    #[test]
    fn own_authority_is_a_loopback_literal_on_the_bound_port() {
        for authority in [
            "127.0.0.1:49152",
            "localhost:49152",
            "[::1]:49152",
            "LOCALHOST:49152",
        ] {
            assert!(is_own_authority(authority, PORT), "{authority} is ours");
        }
        for authority in [
            "127.0.0.1:49153",
            "127.0.0.1",
            "localhost",
            "[::1]",
            "evil.example:49152",
            "127.0.0.1.evil.example:49152",
            "127.0.0.1:+49152",
            "127.0.0.1:049152",
            "",
        ] {
            assert!(
                !is_own_authority(authority, PORT),
                "{authority} is not ours"
            );
        }
    }

    #[test]
    fn host_allowed_needs_exactly_one_own_host_header() {
        assert!(host_allowed(
            &headers_with(&[(HOST, "127.0.0.1:49152")]),
            PORT
        ));
        assert!(
            !host_allowed(&HeaderMap::new(), PORT),
            "a request without Host is refused"
        );
        assert!(
            !host_allowed(&headers_with(&[(HOST, "evil.example:49152")]), PORT),
            "a rebinding page's Host is refused"
        );
        assert!(
            !host_allowed(
                &headers_with(&[(HOST, "127.0.0.1:49152"), (HOST, "evil.example")]),
                PORT
            ),
            "two Host headers are refused, whichever one a parser would read"
        );
    }

    #[test]
    fn origin_allowed_when_absent_or_our_own_http_origin() {
        assert!(
            origin_allowed(&HeaderMap::new(), PORT),
            "a non-browser client sends no Origin"
        );
        assert!(origin_allowed(
            &headers_with(&[(ORIGIN, "http://127.0.0.1:49152")]),
            PORT
        ));
        for origin in [
            "null",
            "https://127.0.0.1:49152",
            "http://evil.example",
            "http://127.0.0.1:49153",
        ] {
            assert!(
                !origin_allowed(&headers_with(&[(ORIGIN, origin)]), PORT),
                "Origin {origin} is refused"
            );
        }
        assert!(
            !origin_allowed(
                &headers_with(&[
                    (ORIGIN, "http://127.0.0.1:49152"),
                    (ORIGIN, "http://evil.example")
                ]),
                PORT
            ),
            "every Origin header must be ours"
        );
    }

    #[tokio::test]
    async fn a_forged_host_is_refused_before_the_token_is_looked_at() {
        let (bridge, _requests) = crate::shell::bridge::channel();
        let endpoint = serve(bridge, Tokens::default(), None).await.expect("bind");
        let headers = "Content-Type: application/json\r\n\
                       Accept: application/json, text/event-stream\r\n";

        let (status, _) =
            round_trip_with_host(&authority(&endpoint), "evil.example", headers, INITIALIZE).await;

        assert!(
            status.contains("403"),
            "termherd refuses the Host itself, not rmcp after the token, got: {status}"
        );
    }

    #[tokio::test]
    async fn a_forged_host_is_refused_even_with_a_valid_token() {
        let (bridge, _requests) = crate::shell::bridge::channel();
        let tokens = Tokens::default();
        let endpoint = serve(bridge, tokens.clone(), None).await.expect("bind");
        let token = tokens.issue();

        let (status, _) = round_trip_with_host(
            &authority(&endpoint),
            "evil.example",
            &call_headers(&token),
            INITIALIZE,
        )
        .await;

        assert!(status.contains("403"), "got status: {status}");
    }

    #[tokio::test]
    async fn a_foreign_origin_is_refused_even_with_a_valid_token() {
        let (bridge, _requests) = crate::shell::bridge::channel();
        let tokens = Tokens::default();
        let endpoint = serve(bridge, tokens.clone(), None).await.expect("bind");
        let token = tokens.issue();
        let headers = format!("{}Origin: http://evil.example\r\n", call_headers(&token));

        let (status, _) = round_trip(&authority(&endpoint), &headers, INITIALIZE).await;

        assert!(
            status.contains("403"),
            "rmcp checks no Origin by default; termherd does, got: {status}"
        );
    }

    #[tokio::test]
    async fn a_localhost_host_with_its_own_origin_still_initializes() {
        let (bridge, _requests) = crate::shell::bridge::channel();
        let tokens = Tokens::default();
        let endpoint = serve(bridge, tokens.clone(), None).await.expect("bind");
        let token = tokens.issue();
        let authority = authority(&endpoint);
        let port = authority
            .rsplit_once(':')
            .map(|(_, port)| port)
            .expect("a port");
        let host = format!("localhost:{port}");
        let headers = format!("{}Origin: http://{host}\r\n", call_headers(&token));

        let (status, _) = round_trip_with_host(&authority, &host, &headers, INITIALIZE).await;

        assert!(status.contains("200"), "got status: {status}");
    }

    #[test]
    fn authorized_only_for_a_minted_bearer_token() {
        let tokens = Tokens::default();
        let token = tokens.issue();
        assert!(
            authorized(&header_map(&format!("Bearer {token}")), &tokens),
            "a minted token in a Bearer header authorizes"
        );
        assert!(
            !authorized(&header_map("Bearer forged-token"), &tokens),
            "a forged token is refused"
        );
        assert!(
            !authorized(&HeaderMap::new(), &tokens),
            "no header is refused"
        );
    }
}
