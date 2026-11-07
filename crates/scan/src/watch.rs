//! Debounced filesystem watch behind live sidebar updates (FR2). A fully
//! independent leaf: the notify/coalesce seam, with no dependency on the walk
//! or its cache.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use termherd_core::ports::ScanError;
use tracing::{debug, warn};

/// Keeps the fs watcher and its coalescing thread alive; dropping it stops
/// both.
pub struct WatchHandle {
    _watcher: notify::RecommendedWatcher,
}

/// Watch `root` recursively and invoke `on_change` once per debounced
/// burst of filesystem events (the CLI appends JSONL lines continuously;
/// without coalescing every keystroke of a session would trigger a
/// rescan). The callback runs on a background thread.
pub fn watch_changes(
    root: PathBuf,
    debounce: Duration,
    on_change: impl FnMut() + Send + 'static,
) -> Result<WatchHandle, ScanError> {
    watch(
        &root,
        RecursiveMode::Recursive,
        |_| true,
        debounce,
        on_change,
    )
}

/// Watch one file and invoke `on_change` once per debounced burst of events
/// that name it. The parent directory is what is watched, not the file: an
/// editor that saves by writing a temporary and renaming it over the original
/// replaces the inode a file watch would hold, and the file may not exist yet.
/// Events about its neighbours are ignored.
pub fn watch_file(
    file: PathBuf,
    debounce: Duration,
    on_change: impl FnMut() + Send + 'static,
) -> Result<WatchHandle, ScanError> {
    let dir = file
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| ScanError::Unreadable(format!("{}: no parent", file.display())))?;
    let name = file.file_name().map(std::ffi::OsStr::to_os_string);
    watch(
        &dir,
        RecursiveMode::NonRecursive,
        move |event| {
            event
                .paths
                .iter()
                .any(|path| path.file_name() == name.as_deref())
        },
        debounce,
        on_change,
    )
}

/// The shared body: watch `root` in `mode`, keep the events `relevant` admits,
/// and coalesce them into one `on_change` per quiet `debounce`.
fn watch(
    root: &Path,
    mode: RecursiveMode,
    relevant: impl Fn(&notify::Event) -> bool + Send + 'static,
    debounce: Duration,
    on_change: impl FnMut() + Send + 'static,
) -> Result<WatchHandle, ScanError> {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        deliver(event, &relevant, || {
            let _ = tx.send(());
        });
    })
    .map_err(|e| ScanError::Unreadable(format!("watcher: {e}")))?;
    watcher
        .watch(root, mode)
        .map_err(|e| ScanError::Unreadable(format!("{}: {e}", root.display())))?;

    std::thread::Builder::new()
        .name("termherd-fs-watch".into())
        .spawn(move || coalesce(&rx, debounce, on_change))
        .map_err(|e| ScanError::Unreadable(format!("watch thread: {e}")))?;

    Ok(WatchHandle { _watcher: watcher })
}

/// One watcher callback: `signal` when the event is one `relevant` admits.
/// An error is logged rather than signalled: a watch that keeps failing
/// would otherwise leave the sidebar stale with no trace of why.
fn deliver(
    event: notify::Result<notify::Event>,
    relevant: impl Fn(&notify::Event) -> bool,
    signal: impl FnOnce(),
) {
    match event {
        Ok(event) if relevant(&event) => signal(),
        Ok(_) => {}
        Err(error) => warn!(%error, paths = ?error.paths, "fs watch error"),
    }
}

/// Turn the stream of signals on `rx` into one `on_change` per burst, until
/// every sender is gone.
fn coalesce(rx: &Receiver<()>, debounce: Duration, mut on_change: impl FnMut()) {
    // One blocking recv starts a burst; keep draining until the
    // tree has been quiet for `debounce`, then fire once.
    while rx.recv().is_ok() {
        while rx.recv_timeout(debounce).is_ok() {}
        on_change();
    }
    debug!("fs watch channel closed; coalescing thread exiting");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Arc, Mutex};

    /// Everything logged at WARN or above while `run` executes.
    fn warnings_during(run: impl FnOnce()) -> String {
        #[derive(Clone, Default)]
        struct Sink(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Sink {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let sink = Sink::default();
        let writer = sink.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, run);
        let bytes = sink.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn a_watcher_error_is_logged_and_signals_nothing() {
        let mut signalled = false;
        let logged = warnings_during(|| {
            deliver(
                Err(notify::Error::generic("stream lost")),
                |_| true,
                || signalled = true,
            );
        });
        assert!(!signalled, "an error is not a change");
        assert!(
            logged.contains("WARN"),
            "not logged as a warning: {logged:?}"
        );
        assert!(
            logged.contains("stream lost"),
            "error text missing: {logged:?}"
        );
    }

    #[test]
    fn a_relevant_event_signals_and_an_irrelevant_one_does_not() {
        let event = || Ok(notify::Event::new(notify::EventKind::Any));
        let mut signals = 0;
        deliver(event(), |_| true, || signals += 1);
        deliver(event(), |_| false, || signals += 1);
        assert_eq!(signals, 1);
    }

    #[test]
    fn watch_fires_once_per_debounced_burst() {
        let tmp = tempfile::tempdir().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let _handle = watch_changes(
            tmp.path().to_owned(),
            std::time::Duration::from_millis(200),
            move || {
                let _ = tx.send(());
            },
        )
        .unwrap();

        // A burst of writes…
        for i in 0..5 {
            fs::write(tmp.path().join(format!("f{i}.jsonl")), "x").unwrap();
        }
        // …yields at least one change signal (fs event latency varies by
        // platform, so allow a generous window but require coalescing to
        // have collapsed the burst into very few signals).
        assert!(
            rx.recv_timeout(std::time::Duration::from_secs(10)).is_ok(),
            "no change signal within 10s"
        );
        let extra =
            std::iter::from_fn(|| rx.recv_timeout(std::time::Duration::from_millis(600)).ok())
                .count();
        assert!(
            extra <= 2,
            "burst was not coalesced: {} extra signals",
            extra + 1
        );
    }

    #[test]
    fn a_file_watch_fires_for_its_file_and_ignores_its_neighbours() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("settings.json");
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let _handle = watch_file(
            file.clone(),
            std::time::Duration::from_millis(100),
            move || {
                let _ = tx.send(());
            },
        )
        .unwrap();

        fs::write(tmp.path().join("metadata.json"), "{}").unwrap();
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(800))
                .is_err(),
            "a neighbour's write must not fire"
        );

        // Written as an editor saves: a temporary renamed over the target.
        let staged = tmp.path().join("settings.json.tmp");
        fs::write(&staged, "{}").unwrap();
        fs::rename(&staged, &file).unwrap();
        assert!(
            rx.recv_timeout(std::time::Duration::from_secs(10)).is_ok(),
            "no change signal for the watched file within 10s"
        );
    }
}
