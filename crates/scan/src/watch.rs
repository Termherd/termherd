//! Debounced filesystem watch behind live sidebar updates (FR2). A fully
//! independent leaf: the notify/coalesce seam, with no dependency on the walk
//! or its cache.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};
use termherd_core::ports::ScanError;
use tracing::{debug, warn};

/// The longest a burst of events can hold back its change signal. Past it the
/// signal fires though events are still arriving: a session writing its
/// transcript non-stop must still refresh the sidebar.
pub const MAX_WAIT: Duration = Duration::from_secs(2);

/// Keeps the fs watcher and its coalescing thread alive; dropping it stops
/// both.
pub struct WatchHandle {
    _watcher: notify::RecommendedWatcher,
}

/// Watch `root` recursively and invoke `on_change` once per debounced
/// burst of filesystem events (the CLI appends JSONL lines continuously;
/// without coalescing every keystroke of a session would trigger a
/// rescan). A burst fires after `debounce` of quiet, or [`MAX_WAIT`] after
/// its first event. The callback runs on a background thread.
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
/// and coalesce them into one `on_change` per burst.
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

/// Turn the stream of signals on `rx` into one `on_change` per burst, as
/// [`Debounce`] times it, until every sender is gone.
fn coalesce(rx: &Receiver<()>, debounce: Duration, mut on_change: impl FnMut()) {
    let mut burst = Debounce::new(debounce, MAX_WAIT);
    loop {
        let received = match burst.deadline() {
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(deadline) => rx.recv_timeout(deadline.saturating_duration_since(Instant::now())),
        };
        match received {
            Ok(()) => burst.event(Instant::now()),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if burst.take_due(Instant::now()) {
            on_change();
        }
    }
    debug!("fs watch channel closed; coalescing thread exiting");
}

/// When a burst of events is due to fire: once `quiet` has passed without an
/// event, or `max_wait` after its first event, whichever comes first. Pure:
/// the caller supplies the clock.
#[derive(Debug)]
struct Debounce {
    quiet: Duration,
    max_wait: Duration,
    burst: Option<Burst>,
}

/// The first and latest event of the burst not yet fired.
#[derive(Debug, Clone, Copy)]
struct Burst {
    first: Instant,
    last: Instant,
}

impl Debounce {
    fn new(quiet: Duration, max_wait: Duration) -> Self {
        Self {
            quiet,
            max_wait,
            burst: None,
        }
    }

    /// Record an event at `now`: it opens a burst, or extends the open one.
    fn event(&mut self, now: Instant) {
        let first = self.burst.map_or(now, |burst| burst.first);
        self.burst = Some(Burst { first, last: now });
    }

    /// When the open burst fires, `None` when there is none.
    fn deadline(&self) -> Option<Instant> {
        self.burst
            .map(|burst| (burst.last + self.quiet).min(burst.first + self.max_wait))
    }

    /// Whether the open burst is due at `now`; a due burst is closed, so it
    /// fires once.
    fn take_due(&mut self, now: Instant) -> bool {
        let due = self.deadline().is_some_and(|deadline| deadline <= now);
        if due {
            self.burst = None;
        }
        due
    }
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

    const QUIET: Duration = Duration::from_millis(500);

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn no_event_means_no_deadline() {
        let mut debounce = Debounce::new(QUIET, MAX_WAIT);
        assert_eq!(debounce.deadline(), None);
        assert!(!debounce.take_due(Instant::now()));
    }

    #[test]
    fn a_lone_event_fires_after_the_quiet_period_once() {
        let base = Instant::now();
        let mut debounce = Debounce::new(QUIET, MAX_WAIT);
        debounce.event(base);
        assert_eq!(debounce.deadline(), Some(at(base, 500)));
        assert!(!debounce.take_due(at(base, 499)));
        assert!(debounce.take_due(at(base, 500)));
        assert!(!debounce.take_due(at(base, 501)), "fired twice");
        assert_eq!(debounce.deadline(), None);
    }

    #[test]
    fn a_new_event_pushes_the_quiet_deadline_back() {
        let base = Instant::now();
        let mut debounce = Debounce::new(QUIET, MAX_WAIT);
        debounce.event(base);
        debounce.event(at(base, 300));
        assert_eq!(debounce.deadline(), Some(at(base, 800)));
    }

    #[test]
    fn a_steady_stream_fires_at_max_wait_after_its_first_event() {
        let base = Instant::now();
        let mut debounce = Debounce::new(QUIET, MAX_WAIT);
        let mut fired = None;
        for ms in (0..5_000).step_by(100) {
            let now = at(base, ms);
            if debounce.take_due(now) {
                fired = Some(ms);
                break;
            }
            debounce.event(now);
        }
        assert_eq!(fired, Some(2_000));
    }

    #[test]
    fn after_firing_the_next_event_opens_a_fresh_burst() {
        let base = Instant::now();
        let mut debounce = Debounce::new(QUIET, MAX_WAIT);
        debounce.event(base);
        assert!(debounce.take_due(at(base, 500)));
        debounce.event(at(base, 1_900));
        assert_eq!(debounce.deadline(), Some(at(base, 2_400)));
    }

    #[test]
    fn coalescing_fires_during_a_steady_stream_and_stops_with_its_senders() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let (fired_tx, fired_rx) = std::sync::mpsc::channel::<Instant>();
        let coalescer = std::thread::spawn(move || {
            coalesce(&rx, QUIET, || {
                let _ = fired_tx.send(Instant::now());
            });
        });
        let start = Instant::now();
        while start.elapsed() < MAX_WAIT + Duration::from_secs(1) {
            tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(50));
        }
        let stream_end = Instant::now();
        drop(tx);
        coalescer.join().unwrap();
        let fired: Vec<Instant> = fired_rx.try_iter().collect();
        assert!(
            fired.iter().any(|at| *at < stream_end),
            "nothing fired while events kept arriving: {fired:?}"
        );
    }

    /// Each burst as `(first event, fire time)`, with the clock jumping
    /// straight to each event and each deadline, as `coalesce` would wait.
    fn bursts(gaps_ms: &[u64]) -> Vec<(Instant, Instant)> {
        let base = Instant::now();
        let mut debounce = Debounce::new(QUIET, MAX_WAIT);
        let mut fired = Vec::new();
        let mut first = None;
        let mut now = base;
        for gap in gaps_ms {
            now += Duration::from_millis(*gap);
            while let Some(deadline) = debounce.deadline().filter(|d| *d <= now) {
                assert!(debounce.take_due(deadline));
                fired.push((first.take().unwrap(), deadline));
            }
            debounce.event(now);
            first.get_or_insert(now);
        }
        if let Some(deadline) = debounce.deadline() {
            assert!(debounce.take_due(deadline));
            fired.push((first.take().unwrap(), deadline));
        }
        fired
    }

    proptest::proptest! {
        #[test]
        fn every_burst_fires_within_max_wait_of_its_first_event(
            gaps in proptest::collection::vec(0u64..3_000, 1..200)
        ) {
            let fired = bursts(&gaps);
            proptest::prop_assert!(!fired.is_empty());
            for (first, fire) in fired {
                proptest::prop_assert!(fire >= first);
                proptest::prop_assert!(fire - first <= MAX_WAIT);
            }
        }
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
