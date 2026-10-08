//! Clicking a desktop notification brings its session back: the `os-notify`
//! thread that posted it waits for the OS's answer, a click sends the session
//! on [`NotificationClicks`], and the shell reveals the pane through the path
//! the MCP `focus_pane` tool takes, then raises the window.
//!
//! **Why a cap on waiting threads.** A notification nobody answers keeps its
//! thread parked — on macOS for as long as the banner sits in the notification
//! centre, on Windows possibly forever once the toast moves to the action
//! centre. Replacing a session's previous waiter is not available instead:
//! the thread is blocked inside the OS backend, and `notify-rust` exposes no
//! way to withdraw a macOS notification, so an "abandoned" waiter would be
//! exactly as parked as a live one. At most [`MAX_WAITING`] notifications are
//! clickable at once; past that they still post, without the click. A waiter
//! frees its slot whenever its notification is answered, dismissed or expires.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use iced::futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use iced::futures::{SinkExt, Stream, StreamExt};
use iced::{Task, window};
use termherd_core::SessionId;

use super::streams::TakeOnceSource;
use super::{Message, Shell};

/// How many posted notifications may wait for a click at once. Each holds one
/// parked thread; this is the ceiling on how many a busy, unattended workspace
/// can accumulate.
pub(super) const MAX_WAITING: usize = 16;

/// The sending half: hands out one [`ClickSlot`] per clickable notification,
/// up to [`MAX_WAITING`] alive at a time.
#[derive(Clone)]
pub(super) struct NotificationClicks {
    tx: UnboundedSender<SessionId>,
    waiting: Arc<AtomicUsize>,
}

/// The receiving half, drained by [`click_stream`].
pub(super) type ClickSource = TakeOnceSource<UnboundedReceiver<SessionId>>;

/// A fresh click channel: the sender the effect executor posts with, and the
/// source the subscription drains.
pub(super) fn channel() -> (NotificationClicks, ClickSource) {
    let (tx, rx) = unbounded();
    (
        NotificationClicks {
            tx,
            waiting: Arc::new(AtomicUsize::new(0)),
        },
        TakeOnceSource::new(rx),
    )
}

impl NotificationClicks {
    /// Reserve a waiter for `session`'s notification, or `None` when
    /// [`MAX_WAITING`] are already parked.
    pub(super) fn reserve(&self, session: SessionId) -> Option<ClickSlot> {
        self.waiting
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_WAITING).then_some(n + 1)
            })
            .ok()
            .map(|_| ClickSlot {
                session,
                tx: self.tx.clone(),
                waiting: Arc::clone(&self.waiting),
            })
    }
}

/// One notification's claim on a waiting thread. Its slot is released when it
/// drops — clicked or not — so a dismissed notification frees it as surely as
/// a clicked one.
pub(super) struct ClickSlot {
    session: SessionId,
    tx: UnboundedSender<SessionId>,
    waiting: Arc<AtomicUsize>,
}

impl ClickSlot {
    /// The user clicked the notification: route its session to the shell.
    pub(super) fn clicked(self) {
        // The receiver only closes when the app is shutting down.
        let _ = self.tx.unbounded_send(self.session);
    }
}

impl Drop for ClickSlot {
    fn drop(&mut self) {
        self.waiting.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The click stream: each clicked session becomes a
/// [`Message::NotificationClicked`].
pub(super) fn click_stream(source: &ClickSource) -> impl Stream<Item = Message> + use<> {
    let taken = source.take();
    iced::stream::channel(
        4,
        |mut out: iced::futures::channel::mpsc::Sender<Message>| async move {
            match taken {
                Some(mut rx) => {
                    while let Some(session) = rx.next().await {
                        if out
                            .send(Message::NotificationClicked(session))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
                None => iced::futures::future::pending::<()>().await,
            }
        },
    )
}

impl Shell {
    /// A notification was clicked: reveal its pane as `focus_pane` would, and
    /// bring the window forward. A session closed since the notification was
    /// posted reveals nothing, but the window still comes forward — the user
    /// asked for termherd, and it is the one thing left to show them.
    pub(super) fn on_notification_clicked(&mut self, session: SessionId) -> Task<Message> {
        let reveal = if self.core.workspace.tab_of(session).is_some() {
            self.reveal_session(session)
        } else {
            tracing::debug!(?session, "clicked notification's session is gone");
            Task::none()
        };
        Task::batch([reveal, window::latest().and_then(window::gain_focus)])
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::*;

    fn session(n: u64) -> SessionId {
        SessionId(NonZeroU64::new(n).expect("non-zero"))
    }

    #[test]
    fn a_clicked_slot_reaches_the_subscription_as_a_message() {
        let (clicks, source) = channel();
        clicks.reserve(session(7)).expect("a free slot").clicked();
        drop(clicks);

        let mut stream = Box::pin(click_stream(&source));
        let message = iced::futures::executor::block_on(stream.next()).expect("one message");
        assert!(
            matches!(message, Message::NotificationClicked(s) if s == session(7)),
            "got {message:?}"
        );
    }

    #[test]
    fn an_unclicked_slot_sends_nothing() {
        let (clicks, source) = channel();
        drop(clicks.reserve(session(7)).expect("a free slot"));
        drop(clicks);

        // A dismissed notification must not reveal anything: the stream ends
        // with no message rather than delivering one.
        let mut stream = Box::pin(click_stream(&source));
        assert!(iced::futures::executor::block_on(stream.next()).is_none());
    }

    #[test]
    fn waiters_are_capped_and_a_released_slot_is_reusable() {
        let (clicks, _source) = channel();
        let mut held: Vec<ClickSlot> = (1..=MAX_WAITING as u64)
            .map(|n| clicks.reserve(session(n)).expect("under the cap"))
            .collect();

        assert!(clicks.reserve(session(99)).is_none(), "the cap holds");

        // Answered or dismissed, a notification gives its slot back.
        held.pop().expect("one held").clicked();
        drop(held.pop());
        let refilled = [clicks.reserve(session(99)), clicks.reserve(session(100))];
        assert!(refilled.iter().all(Option::is_some), "both freed slots");
        assert!(clicks.reserve(session(101)).is_none(), "and no more");
    }
}
