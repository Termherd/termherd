//! termherd — entry point and composition root.
//!
//! Constructs the concrete adapters here and injects them (Q4 — no
//! require-time singletons): tracing, single-instance, the filesystem
//! scanner, then the iced shell.

mod capture;
mod collapsed_store;
mod docs;
#[cfg(target_os = "macos")]
mod macos;
mod metadata_store;
mod paths;
mod record;
mod settings;
mod shell;
mod strings;
mod window_config;

use std::sync::Arc;

use single_instance::SingleInstance;
use termherd_core::ports::{ProjectScanner, PtyHost, ScanError};
use termherd_pty::{EventSink, PtyEvent, PtyManager, Shell};
use termherd_scan::FsScanner;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

/// Base name of the single-instance lock. Its *form* is adjusted per OS by
/// [`lock_name`], because `single-instance` gives this string a different
/// meaning on each platform.
const INSTANCE_LOCK: &str = "dev.termherd.lock";

fn main() -> iced::Result {
    init_tracing();

    // Hold the single-instance guard for the whole GUI lifetime.
    let instance = acquire_single_instance();

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
    let keymap = settings.keymap();
    let metadata = metadata_store::load();
    let collapsed = collapsed_store::load();

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
        collapsed,
        record: settings.record_config(),
        session_limit: settings.session_limit(),
    };
    let result = shell::run(scanner, watch_root, pty, pty_rx, startup);
    // Keep the single-instance guard alive until the GUI exits.
    drop(instance);
    result
}

/// The single-instance lock identifier, in the form each OS requires. The
/// `single-instance` crate gives this one string three different meanings, and
/// a value valid on one platform is broken on another:
///
/// - **macOS** treats it as a *file path* and `flock`s it, so it must be an
///   absolute, writable path. A bare name is created relative to the CWD —
///   which is `/` (read-only) when launched from the `.app` bundle, so the app
///   would silently refuse to start (the "double-click does nothing" bug).
/// - **Windows** passes it to `CreateMutexW` as a *mutex name*, which must not
///   contain `\` (those are reserved namespace separators). A full path there
///   yields `ERROR_PATH_NOT_FOUND` (3) and the lock silently disables itself.
///   A bare name lands in the per-login-session namespace — the scope we want.
/// - **Linux** binds it as an *abstract socket name* — any opaque string works.
fn lock_name() -> std::borrow::Cow<'static, str> {
    #[cfg(target_os = "macos")]
    {
        std::borrow::Cow::Owned(
            std::env::temp_dir()
                .join(INSTANCE_LOCK)
                .to_string_lossy()
                .into_owned(),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::borrow::Cow::Borrowed(INSTANCE_LOCK)
    }
}

/// Acquire the single-instance guard, returning it to hold for the process
/// lifetime — or `None` when the lock subsystem is unavailable.
///
/// Exits the process only when another instance already holds the lock. A
/// failure to *create* the lock must not stop the app from launching: that was
/// the "double-click does nothing, no error" bug on the `.app` bundle.
fn acquire_single_instance() -> Option<SingleInstance> {
    let name = lock_name();
    match SingleInstance::new(&name) {
        Ok(instance) => {
            if !instance.is_single() {
                warn!("another termherd instance is already running; exiting");
                // Non-zero so a launcher can tell this from a clean shutdown.
                std::process::exit(2);
            }
            Some(instance)
        }
        Err(e) => {
            warn!(
                error = %e, lock = %name,
                "single-instance lock unavailable; launching without it"
            );
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

/// Default tracing filter: our crates at `info`; the iced/wgpu/winit stack
/// pinned to `warn` because it dumps verbose `info` startup blocks (full
/// `WindowAttributes`, compositor settings, adapter lists) through `tracing`,
/// which otherwise floods the terminal. `RUST_LOG` overrides this when set.
const DEFAULT_FILTER: &str = "info,\
    iced_winit=warn,iced_wgpu=warn,wgpu_core=warn,wgpu_hal=warn,\
    naga=warn,cosmic_text=warn,winit=warn";

fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_FILTER;
    use tracing_subscriber::EnvFilter;

    #[cfg(target_os = "macos")]
    #[test]
    fn lock_name_is_absolute_on_macos() {
        // A CWD-relative lock path is the Finder/.app launch bug: from `/`
        // (read-only) it can't be created and the app refuses to start.
        assert!(
            std::path::Path::new(super::lock_name().as_ref()).is_absolute(),
            "macOS single-instance lock path must be CWD-independent"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn lock_name_has_no_backslash_on_windows() {
        // A Windows mutex name treats `\` as a namespace separator: a path
        // yields ERROR_PATH_NOT_FOUND (3) and silently disables the lock.
        assert!(
            !super::lock_name().contains('\\'),
            "Windows single-instance mutex name must not contain backslashes"
        );
    }

    #[test]
    fn default_filter_parses_cleanly() {
        // A typo would make `EnvFilter` silently drop the bad directive and
        // re-enable the dependency flood (#11); fail the build instead.
        let filter = EnvFilter::builder().parse(DEFAULT_FILTER);
        assert!(filter.is_ok(), "DEFAULT_FILTER must be valid: {filter:?}");
    }
}
