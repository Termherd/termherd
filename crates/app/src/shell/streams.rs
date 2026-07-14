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

/// A receiver wrapped so an iced `Subscription` can take it exactly once and
/// stay stable across `view`/`update` cycles: the `Arc` identity is the hash
/// (so the subscription is not recreated every cycle), and the receiver is
/// moved out on the stream's first — and only — run. Shared by every
/// single-consumer subscription source (PTY output here, the async bridge in
/// [`super::bridge`]).
pub struct TakeOnceSource<R>(Arc<Mutex<Option<R>>>);

impl<R> TakeOnceSource<R> {
    pub(super) fn new(rx: R) -> Self {
        Self(Arc::new(Mutex::new(Some(rx))))
    }

    /// Take the receiver: `Some` on the first call, `None` after — a duplicated
    /// subscription must idle rather than steal events.
    pub(super) fn take(&self) -> Option<R> {
        self.0.lock().ok().and_then(|mut slot| slot.take())
    }
}

// Manual `Clone`/`Hash`: the receiver `R` is neither, but the wrapper only ever
// clones the `Arc` and hashes its pointer, so no `R: Clone`/`R: Hash` bound.
impl<R> Clone for TakeOnceSource<R> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<R> Hash for TakeOnceSource<R> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.0) as usize).hash(state);
    }
}

/// Streams PTY output/exit into the subscription.
pub(super) type PtyOutput = TakeOnceSource<UnboundedReceiver<PtyEvent>>;

/// The [`Message`] a [`PtyEvent`] becomes — the adapter→shell glue, shared by
/// the subscription stream and the end-to-end tests that pump events by hand.
pub(super) fn pty_message(event: PtyEvent) -> Message {
    match event {
        PtyEvent::Output { session, screen } => Message::PtyOutput { session, screen },
        PtyEvent::Status { session, status } => Message::PtyStatus { session, status },
        PtyEvent::Title { session, title } => Message::PtyTitle { session, title },
        PtyEvent::Notification { session, body } => Message::PtyNotify { session, body },
        PtyEvent::Exited { session, clean } => Message::PtyExited { session, clean },
        // The clipboard is global, so the requesting session no longer
        // matters — reuse the copy-to-clipboard path.
        PtyEvent::SelectionCopied { text, .. } => Message::CopySelection(text),
    }
}

/// One PTY-output stream: drains the receiver into [`Message`]s. The receiver
/// is taken on first run; a duplicated subscription (there is only ever one)
/// would idle forever rather than steal events.
pub(super) fn pty_stream(output: &PtyOutput) -> impl Stream<Item = Message> + use<> {
    let taken = output.take();
    iced::stream::channel(
        64,
        |mut out: iced::futures::channel::mpsc::Sender<Message>| async move {
            match taken {
                Some(mut rx) => {
                    while let Some(event) = rx.next().await {
                        if out.send(pty_message(event)).await.is_err() {
                            break;
                        }
                    }
                }
                None => iced::futures::future::pending::<()>().await,
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
