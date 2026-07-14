//! Debounced filesystem watch behind live sidebar updates (FR2). A fully
//! independent leaf: the notify/coalesce seam, with no dependency on the walk
//! or its cache.

use std::path::PathBuf;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use termherd_core::ports::ScanError;
use tracing::debug;

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
    mut on_change: impl FnMut() + Send + 'static,
) -> Result<WatchHandle, ScanError> {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok() {
            let _ = tx.send(());
        }
    })
    .map_err(|e| ScanError::Unreadable(format!("watcher: {e}")))?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| ScanError::Unreadable(format!("{}: {e}", root.display())))?;

    std::thread::Builder::new()
        .name("termherd-fs-watch".into())
        .spawn(move || {
            // One blocking recv starts a burst; keep draining until the
            // tree has been quiet for `debounce`, then fire once.
            while rx.recv().is_ok() {
                while rx.recv_timeout(debounce).is_ok() {}
                on_change();
            }
            debug!("fs watch channel closed; coalescing thread exiting");
        })
        .map_err(|e| ScanError::Unreadable(format!("watch thread: {e}")))?;

    Ok(WatchHandle { _watcher: watcher })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
}
