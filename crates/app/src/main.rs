//! termherd — entry point and composition root.
//!
//! Constructs the concrete adapters here and injects them (Q4 — no
//! require-time singletons): tracing, single-instance, the filesystem
//! scanner, then the iced shell.

mod metadata_store;
mod settings;
mod shell;
mod window_config;

use std::sync::Arc;

use single_instance::SingleInstance;
use termherd_core::ports::{ProjectScanner, PtyHost, ScanError};
use termherd_pty::{EventSink, PtyEvent, PtyManager, Shell};
use termherd_scan::FsScanner;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const INSTANCE_ID: &str = "dev.gallay.termherd";

fn main() -> iced::Result {
    init_tracing();

    let instance = match SingleInstance::new(INSTANCE_ID) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "could not acquire single-instance lock");
            std::process::exit(1);
        }
    };
    if !instance.is_single() {
        warn!("another termherd instance is already running; exiting");
        // Non-zero so a launcher can tell the difference from a clean shutdown.
        std::process::exit(2);
    }

    info!(
        version = env!("CARGO_PKG_VERSION"),
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
    let keymap = settings.keymap();
    let metadata = metadata_store::load();

    // PTY output flows from the reader threads through this channel into the
    // iced subscription (M2). The manager is built here and injected as a
    // `dyn PtyHost` — no global state (Q4).
    let (tx, pty_rx) = iced::futures::channel::mpsc::unbounded::<PtyEvent>();
    let sink: EventSink = Arc::new(move |event| {
        let _ = tx.unbounded_send(event);
    });
    let pty: Arc<dyn PtyHost> = Arc::new(PtyManager::new(sink, shell));

    let startup = shell::Startup {
        theme: settings.theme,
        keymap,
        metadata,
    };
    let result = shell::run(scanner, watch_root, pty, pty_rx, startup);
    // Hold the lock for the whole GUI lifetime.
    drop(instance);
    result
}

/// Fallback scanner when no home directory exists — an empty browser is
/// better than refusing to start.
struct NoScanner;

impl ProjectScanner for NoScanner {
    fn scan(&self) -> Result<Vec<termherd_core::SessionRecord>, ScanError> {
        Ok(Vec::new())
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}
