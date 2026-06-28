//! Subscription stream sources (ARCHITECTURE §8): the PTY-output channel and
//! the fs-watch channel, each wrapped so an iced `Subscription` can drive it
//! and stay stable across `view`/`update` cycles.

use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::futures::channel::mpsc::UnboundedReceiver;
use iced::futures::{SinkExt, Stream, StreamExt};
use termherd_pty::PtyEvent;

use super::Message;

/// Quiet period before a burst of fs events triggers one rescan.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(500);

/// Streams PTY output/exit into the subscription. Wraps the channel receiver
/// so it can be moved into the stream once; the `Arc` identity makes the
/// subscription stable across `view`/`update` cycles (it hashes by pointer).
#[derive(Clone)]
pub(super) struct PtyOutput(Arc<Mutex<Option<UnboundedReceiver<PtyEvent>>>>);

impl PtyOutput {
    pub(super) fn new(rx: UnboundedReceiver<PtyEvent>) -> Self {
        Self(Arc::new(Mutex::new(Some(rx))))
    }
}

impl Hash for PtyOutput {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.0) as usize).hash(state);
    }
}

/// One PTY-output stream: drains the receiver into [`Message`]s. The receiver
/// is taken on first run; a duplicated subscription (there is only ever one)
/// would idle forever rather than steal events.
pub(super) fn pty_stream(output: &PtyOutput) -> impl Stream<Item = Message> + use<> {
    let taken = output.0.lock().ok().and_then(|mut slot| slot.take());
    iced::stream::channel(
        64,
        |mut out: iced::futures::channel::mpsc::Sender<Message>| async move {
            match taken {
                Some(mut rx) => {
                    while let Some(event) = rx.next().await {
                        let message = match event {
                            PtyEvent::Output { session, screen } => {
                                Message::PtyOutput { session, screen }
                            }
                            PtyEvent::Status { session, status } => {
                                Message::PtyStatus { session, status }
                            }
                            PtyEvent::Title { session, title } => {
                                Message::PtyTitle { session, title }
                            }
                            PtyEvent::Notification { session, body } => {
                                Message::PtyNotify { session, body }
                            }
                            PtyEvent::Exited { session } => Message::PtyExited(session),
                        };
                        if out.send(message).await.is_err() {
                            break;
                        }
                    }
                }
                None => iced::futures::future::pending::<()>().await,
            }
        },
    )
}

/// Drives the screencast frame timer (#124). A background thread ticks every
/// `interval`; each tick becomes a [`Message::RecordTick`]. The subscription is
/// present only while recording, so dropping it tears the thread down — its
/// `send` fails once the receiver is gone. A plain `std::thread::sleep` loop,
/// so no async-runtime feature is needed (iced's default backend has no timer).
#[derive(Clone, Hash)]
pub(super) struct RecordTicker {
    pub(super) interval: Duration,
}

/// One frame-timer stream: a thread sleeps `interval` and pushes a tick, the
/// async half forwards each as a [`Message::RecordTick`] until the stream is
/// dropped (recording stopped).
pub(super) fn record_tick_stream(ticker: &RecordTicker) -> impl Stream<Item = Message> + use<> {
    let interval = ticker.interval;
    iced::stream::channel(
        4,
        move |mut output: iced::futures::channel::mpsc::Sender<Message>| async move {
            let (tx, mut rx) = iced::futures::channel::mpsc::unbounded::<()>();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(interval);
                    // Receiver gone (subscription dropped) → stop ticking.
                    if tx.unbounded_send(()).is_err() {
                        break;
                    }
                }
            });
            while rx.next().await.is_some() {
                if output.send(Message::RecordTick).await.is_err() {
                    break;
                }
            }
        },
    )
}

/// One fs-watch stream per projects root: forwards each debounced change
/// burst as a [`Message::ProjectsChanged`]. The watcher lives as long as
/// the stream; if it cannot start, the sidebar simply stops live-updating
/// (logged, not fatal).
// `&PathBuf` is imposed by `Subscription::run_with`, which passes `&D` to a
// plain fn pointer — `&Path` would not match `for<'a> fn(&'a D)`.
#[allow(clippy::ptr_arg)]
pub(super) fn watch_stream(root: &PathBuf) -> impl Stream<Item = Message> + use<> {
    let root = root.clone();
    iced::stream::channel(
        4,
        |mut output: iced::futures::channel::mpsc::Sender<Message>| async move {
            let (tx, mut rx) = iced::futures::channel::mpsc::unbounded::<()>();
            match termherd_scan::watch_changes(root, WATCH_DEBOUNCE, move || {
                let _ = tx.unbounded_send(());
            }) {
                Ok(handle) => {
                    while rx.next().await.is_some() {
                        if output.send(Message::ProjectsChanged).await.is_err() {
                            break;
                        }
                    }
                    drop(handle);
                }
                Err(error) => {
                    tracing::warn!(%error, "fs watch unavailable; sidebar will not live-update");
                    iced::futures::future::pending::<()>().await;
                }
            }
        },
    )
}
