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

    let startup =
        shell::Startup::from_settings(&settings, metadata_store::load(), collapsed_store::load());
    let result = shell::run(scanner, watch_root, pty, pty_rx, startup);
    // Keep the single-instance guard alive until the GUI exits.
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
