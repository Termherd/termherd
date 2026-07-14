//! termherd — entry point and composition root.
//!
//! Constructs the concrete adapters here and injects them (Q4 — no
//! require-time singletons): tracing, single-instance, the filesystem
//! scanner, then the iced shell. Pure wiring — the pieces live in their own
//! modules ([`instance`], [`tracing_init`], the stores).

mod capture;
mod collapsed_store;
mod docs;
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
mod strings;
mod tracing_init;
mod window_config;
mod window_geometry;

use std::sync::Arc;

use termherd_core::ports::{ProjectScanner, PtyHost, ScanError};
use termherd_pty::{EventSink, PtyEvent, PtyManager, Shell};
use termherd_scan::FsScanner;
use tracing::{info, warn};

fn main() -> iced::Result {
    tracing_init::init_tracing();

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

    let live_bridge = shell::LiveBridge {
        requests: bridge_requests,
        mcp_endpoint,
        mcp_tokens,
    };
    let startup =
        shell::Startup::from_settings(&settings, metadata_store::load(), collapsed_store::load());
    let result = shell::run(scanner, watch_root, pty, pty_rx, live_bridge, startup);
    // Keep the single-instance guard and the async substrate alive until the GUI
    // exits: dropping the bridge handle would close the request channel, and
    // dropping the runtime would tear down any transport task hosted on it.
    drop(bridge_handle);
    drop(bridge_runtime);
    drop(instance);
    result
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
