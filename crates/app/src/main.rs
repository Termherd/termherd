//! termherd — entry point and composition root.
//!
//! Constructs the concrete adapters here and injects them (Q4 — no
//! require-time singletons): tracing, single-instance, the filesystem
//! scanner, then the iced shell.

mod shell;
mod window_config;

use std::sync::Arc;

use single_instance::SingleInstance;
use termherd_core::ports::{ProjectScanner, ScanError};
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

    let scanner: Arc<dyn ProjectScanner> = match FsScanner::claude_default() {
        Some(s) => Arc::new(s),
        None => {
            warn!("no home directory found; session browser will be empty");
            Arc::new(NoScanner)
        }
    };

    let result = shell::run(scanner);
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
