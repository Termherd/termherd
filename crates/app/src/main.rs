//! termherd — entry point and composition root.
//!
//! Constructs the concrete adapters here and injects them (Q4 — no
//! require-time singletons): tracing, single-instance, the filesystem
//! scanner, then the iced shell. Pure wiring — the pieces live in their own
//! modules ([`instance`], [`tracing_init`], the stores).

mod attach;
mod capture;
mod cli;
mod collapsed_store;
mod docs;
mod image;
mod instance;
mod json_store;
#[cfg(target_os = "macos")]
mod macos;
mod mcp;
mod metadata_store;
mod paths;
mod record;
mod record_config;
mod settings;
mod shell;
mod snapshot_dto;
mod strings;
mod tracing_init;
mod window_config;
mod window_geometry;

use std::sync::Arc;

use termherd_core::ports::{PathResolver, ProjectScanner, PtyHost, ScanError};
use termherd_pty::{EventSink, PtyEvent, PtyManager, Shell};
use termherd_scan::FsScanner;
use tracing::{info, warn};

fn main() -> iced::Result {
    tracing_init::init_tracing();

    // Answer a question about the *running* instance before touching anything
    // that would disturb it: the single-instance lock below exits(2) on a second
    // process, which is the wrong answer to `termherd mcp-config`.
    if cli::parse(std::env::args()) == cli::Command::McpConfig {
        return report_mcp_config();
    }

    // Hold the single-instance guard for the whole GUI lifetime.
    let instance = instance::acquire_single_instance();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        built = env!("TERMHERD_BUILD_DATE"),
        "termherd starting (M1 browser)"
    );

    let (scanner, watch_root): (Arc<dyn ProjectScanner>, Option<std::path::PathBuf>) =
        match FsScanner::claude_default() {
            Some(s) => {
                let root = s.root().to_owned();
                (Arc::new(s), Some(root))
            }
            None => {
                warn!("no home directory found; session browser will be empty");
                (Arc::new(NoScanner), None)
            }
        };

    // Thin user settings (FR10): the configured shell is injected into the PTY
    // host, the theme into the iced shell. A corrupt file falls back to
    // defaults rather than blocking startup.
    let settings = settings::Settings::load();
    let shell = settings.shell.as_ref().map(|s| Shell {
        program: s.program.clone(),
        args: s.args.clone(),
    });

    // PTY output flows from the reader threads through this channel into the
    // iced subscription (M2). The manager is built here and injected as a
    // `dyn PtyHost` — no global state (Q4).
    let (tx, pty_rx) = iced::futures::channel::mpsc::unbounded::<PtyEvent>();
    let sink: EventSink = Arc::new(move |event| {
        let _ = tx.unbounded_send(event);
    });
    let pty: Arc<dyn PtyHost> = Arc::new(PtyManager::new(sink, shell, settings.palette()));

    // Async transport substrate (composition root only): a tokio runtime to host
    // future transport tasks, and the bridge channel that carries their requests
    // into the shell. No transport is wired yet — this lays the runtime and the
    // request path so the live bridge can attach to them without touching
    // `core`. Both are held for the process lifetime, like the single-instance
    // guard; a runtime that fails to build is non-fatal (the bridge is simply
    // unavailable until the app is restarted).
    let bridge_runtime = build_bridge_runtime();
    let (bridge_handle, bridge_requests) = shell::bridge_channel();

    // Bind the in-process MCP server on the substrate runtime (the live-bridge
    // gate). Non-fatal: with no runtime, or a bind failure, the browser still
    // runs — hosted sessions simply can't reach the live bridge until restart.
    // The token registry is shared with the shell, which mints one per Claude
    // launch and injects it into that session's `mcpServers` config.
    let mcp_tokens = mcp::Tokens::default();
    let mcp_endpoint = bridge_runtime.as_ref().and_then(|runtime| {
        match runtime.block_on(mcp::serve(bridge_handle.clone(), mcp_tokens.clone())) {
            Ok(endpoint) => {
                info!(url = %endpoint.url, "mcp live bridge listening");
                Some(endpoint)
            }
            Err(error) => {
                warn!(%error, "mcp live bridge unavailable; hosted sessions can't reach it");
                None
            }
        }
    });

    // Publish the endpoint for a client termherd did not spawn, if the user
    // asked for it. After the single-instance lock, so the file always describes
    // the instance that is actually running.
    let published = mcp_endpoint
        .as_ref()
        .and_then(|endpoint| publish_bridge(&settings, endpoint, &mcp_tokens));

    let live_bridge = shell::LiveBridge {
        requests: bridge_requests,
        mcp_endpoint,
        mcp_tokens,
    };
    let startup =
        shell::Startup::from_settings(&settings, metadata_store::load(), collapsed_store::load());
    // Terminal path candidates are checked against the real filesystem — the
    // one thing that tells `src/main.rs` from prose like `and/or`.
    let path_resolver: Arc<dyn PathResolver> = Arc::new(termherd_scan::FsPathResolver::new());
    let result = shell::run(
        scanner,
        watch_root,
        path_resolver,
        pty,
        pty_rx,
        live_bridge,
        startup,
    );
    // Stop offering an endpoint that no longer exists. A crash skips this, which
    // is why `termherd mcp-config` probes rather than trusting the file.
    if let Some(path) = published
        && let Err(error) = attach::withdraw(&path)
    {
        warn!(%error, "could not withdraw the published mcp bridge");
    }
    // Keep the single-instance guard and the async substrate alive until the GUI
    // exits: dropping the bridge handle would close the request channel, and
    // dropping the runtime would tear down any transport task hosted on it.
    drop(bridge_handle);
    drop(bridge_runtime);
    drop(instance);
    result
}

/// Publish `{ url, token }` for an outside client, if the setting (and any
/// environment override) asks for it. Returns the path to withdraw at teardown.
///
/// The token is minted here and never revoked while the app runs: it is the
/// instance's own, not a session's, so it outlives every session.
fn publish_bridge(
    settings: &settings::Settings,
    endpoint: &mcp::Endpoint,
    tokens: &mcp::Tokens,
) -> Option<std::path::PathBuf> {
    let env = std::env::var(attach::ATTACH_ENV).ok();
    let verdict = attach::decide(settings.mcp.attach, env.as_deref());
    if verdict.override_refused {
        info!(
            variable = attach::ATTACH_ENV,
            "mcp.attach is \"never\"; the environment override is ignored"
        );
    }
    if !verdict.publish {
        return None;
    }
    let path = attach::bridge_path()?;
    let published = attach::Published {
        url: endpoint.url.clone(),
        token: tokens.issue(),
    };
    match attach::publish(&path, &published) {
        Ok(()) => {
            info!(path = %path.display(), "mcp live bridge published for outside clients");
            Some(path)
        }
        Err(error) => {
            warn!(%error, "could not publish the mcp bridge; outside clients can't attach");
            None
        }
    }
}

/// Print the running instance's `mcpServers` snippet, or say why there is none
/// and exit non-zero so a script can tell the two apart.
///
/// Written straight to the streams rather than through `tracing`: this is a
/// subcommand whose *product* is its stdout, and a log line is neither
/// pipeable nor guaranteed to be enabled. Nothing else in the binary prints.
fn report_mcp_config() -> iced::Result {
    use std::io::Write;

    match cli::mcp_config() {
        Ok(snippet) => {
            let mut out = std::io::stdout();
            let _ = writeln!(out, "{snippet}");
            let _ = out.flush();
            Ok(())
        }
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "termherd: {}", cli::explain(&error));
            std::process::exit(1)
        }
    }
}

/// Build the tokio runtime that hosts the in-process MCP server and the async
/// transport tasks. One worker thread (the substrate is latency-bound plumbing,
/// not throughput work), with both drivers enabled: **time** so
/// `tokio::time::timeout` can bound every bridge call, and **io** for the
/// loopback `TcpListener` the MCP server binds. `None` — logged, never fatal —
/// when the runtime cannot be created.
fn build_bridge_runtime() -> Option<tokio::runtime::Runtime> {
    match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("termherd-bridge")
        .enable_all()
        .build()
    {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            warn!(%error, "async transport runtime unavailable; bridge disabled");
            None
        }
    }
}

/// Fallback scanner when no home directory exists — an empty browser is
/// better than refusing to start.
struct NoScanner;

impl ProjectScanner for NoScanner {
    fn scan(&self) -> Result<Vec<termherd_core::SessionRecord>, ScanError> {
        Ok(Vec::new())
    }
}
