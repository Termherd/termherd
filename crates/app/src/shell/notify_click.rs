//! Clicking a desktop notification brings its session back: the `os-notify`
//! thread that posted it waits for the OS's answer, a click sends the session
//! on [`NotificationClicks`], and the shell reveals the pane through the path
//! the MCP `focus_pane` tool takes, then raises the window.
//!
//! **Why waiting threads are bounded, and how.** A waiting thread is blocked
//! inside the OS backend and nothing can cancel it from outside, so each one
//! lives until its notification is answered, dismissed or expires:
//!
//! - **XDG** reports the notification's id, so a session's next notification
//!   *replaces* the previous one in place and reuses its waiter: one waiter
//!   per session, however chatty the session is.
//! - **macOS** cannot replace a notification, so each one has its own waiter,
//!   alive for as long as the banner sits in the notification centre.
//! - **Windows** answers `Closed(Expired)` when a toast times out into the
//!   action centre, so its waiter returns within seconds — and a later click
//!   from the action centre is lost.
//!
//! Across all of them, at most [`MAX_WAITING`] notifications wait at once;
//! past that they still post, without the click.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use iced::futures::Stream;
use iced::futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use iced::{Task, window};
use termherd_core::SessionId;

use super::streams::{TakeOnceSource, drain_stream};
use super::{Message, Shell};

/// How many posted notifications may wait for a click at once. Each holds one
/// parked thread; this is the ceiling on how many a busy, unattended workspace
/// can accumulate.
pub(super) const MAX_WAITING: usize = 16;

/// The sending half: decides, per notification, how it is posted.
#[derive(Clone)]
pub(super) struct NotificationClicks {
    tx: UnboundedSender<SessionId>,
    live: Arc<Mutex<Live>>,
}

/// The waiters alive now, shared with every [`ClickSlot`].
#[derive(Default)]
struct Live {
    waiting: usize,
    next_waiter: u64,
    /// The notification a session's next one can replace in place: the waiter
    /// that owns it, and the id the OS gave it.
    replaceable: HashMap<SessionId, (u64, u32)>,
}

/// How the effect executor posts one notification.
pub(super) enum Posting {
    /// Post it and wait for the answer on its own thread.
    Wait(ClickSlot),
    /// Replace the session's notification with this id; the waiter already
    /// parked on it carries the click.
    Replace(u32),
    /// [`MAX_WAITING`] are parked: post it without a click.
    Plain,
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
            live: Arc::default(),
        },
        TakeOnceSource::new(rx),
    )
}

/// A panic elsewhere must not stop notifications: the counts stay usable.
fn lock(live: &Mutex<Live>) -> MutexGuard<'_, Live> {
    live.lock().unwrap_or_else(PoisonError::into_inner)
}

impl NotificationClicks {
    /// How `session`'s next notification is posted.
    pub(super) fn posting(&self, session: SessionId) -> Posting {
        let mut live = lock(&self.live);
        if let Some(&(_, id)) = live.replaceable.get(&session) {
            return Posting::Replace(id);
        }
        if live.waiting >= MAX_WAITING {
            return Posting::Plain;
        }
        live.waiting += 1;
        live.next_waiter += 1;
        Posting::Wait(ClickSlot {
            session,
            waiter: live.next_waiter,
            tx: self.tx.clone(),
            live: Arc::clone(&self.live),
        })
    }
}

/// One notification's claim on a waiting thread. Its slot is released when it
/// drops — clicked or not — so a dismissed notification frees it as surely as
/// a clicked one.
pub(super) struct ClickSlot {
    session: SessionId,
    waiter: u64,
    tx: UnboundedSender<SessionId>,
    live: Arc<Mutex<Live>>,
}

impl ClickSlot {
    /// The OS showed the notification under `id` and can replace it in place,
    /// so the session's next notification reuses this waiter.
    #[allow(
        dead_code,
        reason = "only XDG replaces a notification in place, so only the XDG path in `effects::os` calls this"
    )]
    pub(super) fn shown(&self, id: u32) {
        lock(&self.live)
            .replaceable
            .insert(self.session, (self.waiter, id));
    }

    /// The user clicked the notification: route its session to the shell.
    pub(super) fn clicked(self) {
        // The receiver only closes when the app is shutting down.
        let _ = self.tx.unbounded_send(self.session);
    }
}

impl Drop for ClickSlot {
    fn drop(&mut self) {
        let mut live = lock(&self.live);
        live.waiting -= 1;
        if live
            .replaceable
            .get(&self.session)
            .is_some_and(|&(waiter, _)| waiter == self.waiter)
        {
            live.replaceable.remove(&self.session);
        }
    }
}

/// The click stream: each clicked session becomes a
/// [`Message::NotificationClicked`].
pub(super) fn click_stream(source: &ClickSource) -> impl Stream<Item = Message> + use<> {
    drain_stream(source, 4, Message::NotificationClicked)
}

/// Bring the window to the front. `gain_focus` alone does nothing to a
/// minimised window on macOS or Windows, so it is restored first.
fn raise_window() -> Task<Message> {
    window::latest().and_then(|id| window::minimize(id, false).chain(window::gain_focus(id)))
}

impl Shell {
    /// A notification was clicked: reveal its pane as `focus_pane` would, and
    /// bring the window forward. The window comes forward even when nothing is
    /// revealed — the user asked for termherd:
    ///
    /// - a session closed since the notification was posted has no pane;
    /// - an open prompt (a close or quit confirmation, the settings panel, the
    ///   doc editor) keeps the screen it is about, rather than having a
    ///   different tab switched in beneath it. A rename is not such a prompt:
    ///   the click dismisses it, as any click elsewhere does.
    pub(super) fn on_notification_clicked(&mut self, session: SessionId) -> Task<Message> {
        let reveal = if let Some(owner) = self.keyboard_owner() {
            tracing::debug!(prompt = owner.label(), "notification click: prompt open");
            Task::none()
        } else {
            self.reveal_session(session).unwrap_or_else(Task::none)
        };
        Task::batch([reveal, raise_window()])
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use iced::futures::StreamExt;

    use super::*;

    fn session(n: u64) -> SessionId {
        SessionId(NonZeroU64::new(n).expect("non-zero"))
    }

    fn waiter(clicks: &NotificationClicks, n: u64) -> ClickSlot {
        match clicks.posting(session(n)) {
            Posting::Wait(slot) => slot,
            Posting::Replace(_) => panic!("expected a waiter, got a replacement"),
            Posting::Plain => panic!("expected a waiter, got a plain posting"),
        }
    }

    #[test]
    fn a_clicked_slot_reaches_the_subscription_as_a_message() {
        let (clicks, source) = channel();
        waiter(&clicks, 7).clicked();
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
        drop(waiter(&clicks, 7));
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
            .map(|n| waiter(&clicks, n))
            .collect();

        assert!(matches!(clicks.posting(session(99)), Posting::Plain));

        // Answered or dismissed, a notification gives its slot back.
        held.pop().expect("one held").clicked();
        drop(held.pop());
        let _refilled = [waiter(&clicks, 99), waiter(&clicks, 100)];
        assert!(matches!(clicks.posting(session(101)), Posting::Plain));
    }

    #[test]
    fn a_session_with_a_replaceable_notification_reuses_its_waiter() {
        let (clicks, _source) = channel();
        let first = waiter(&clicks, 7);
        first.shown(42);

        // However often the session notifies, it holds one waiter.
        for _ in 0..(MAX_WAITING * 2) {
            assert!(matches!(clicks.posting(session(7)), Posting::Replace(42)));
        }
        // And it costs the other sessions nothing.
        let _other = waiter(&clicks, 8);

        // Once that notification is answered, the next one waits afresh.
        drop(first);
        let _next = waiter(&clicks, 7);
    }

    #[test]
    fn a_superseded_waiter_does_not_forget_its_successor() {
        let (clicks, _source) = channel();
        // Two waiters for one session — the first was never shown under an id
        // the OS can replace, as on macOS — and the second one is.
        let first = waiter(&clicks, 7);
        let second = waiter(&clicks, 7);
        second.shown(42);

        drop(first);
        assert!(matches!(clicks.posting(session(7)), Posting::Replace(42)));
        drop(second);
    }
}
