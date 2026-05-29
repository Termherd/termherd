//! termherd — entry point.
//!
//! M0: initialise tracing, enforce single-instance, log a startup line.
//! M1+: wire concrete adapters into `termherd_core::App` and launch the GUI.

use single_instance::SingleInstance;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const INSTANCE_ID: &str = "dev.gallay.termherd";

fn main() {
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
        "termherd starting (M0 scaffold)"
    );

    // TODO M1: construct adapters, build core::App, run iced event loop.
    let _app = termherd_core::App::new();
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}
