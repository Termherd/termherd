//! The async transport substrate: a timeout-bounded request/reply bridge from
//! an off-thread transport task into the iced-owned `core::App`.
//!
//! `core::App` is pure and single-threaded — the iced shell owns it and applies
//! events on the UI thread; there is no shared `&mut App`. So an external
//! transport (a future socket/HTTP listener, wired later) cannot call it
//! directly. Instead it hands a [`Request`] plus a private reply channel across
//! a bounded channel; the shell drains it in `update`, reads state, and answers
//! on the reply channel. Every call is wrapped in `tokio::time::timeout`, so a
//! shell that never answers (the `openDiff`-style hang) fails the *caller* fast
//! rather than blocking it forever.
//!
//! Only the caller side needs the tokio runtime (for the time driver behind
//! `timeout`); the receiver side is driven by iced's own executor, so the two
//! runtimes meet only at the runtime-agnostic channels.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::futures::{SinkExt, Stream};
use termherd_core::App;
use tokio::sync::{mpsc, oneshot};

use super::Message;

/// Bounded so a burst of transport requests applies backpressure to the caller
/// instead of growing memory without limit.
const CAPACITY: usize = 32;

/// What an external transport can ask the running app. One read-only variant
/// today — the substrate proves the round-trip and the timeout, not a rich
/// surface; the live bridge grows this later.
// Caller-side substrate: constructed and called by tests now, and in production
// by the live-bridge transport wired next — so it reads as dead in the binary
// until that transport lands. The receiver half (`respond`, `snapshot`,
// `request_stream`) is already live via the subscription and `update`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// A read-only snapshot of the workspace.
    Snapshot,
}

/// The app's answer to a [`Request`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Snapshot(Snapshot),
}

/// A minimal read of the workspace: how many tabs are open and which one is
/// active (`None` when the workspace is empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub tab_count: usize,
    pub active_tab: Option<usize>,
}

/// Why a bridge call returned no reply. Kept distinct so a caller can tell a
/// timeout (shell alive but slow) from a closed bridge (shell gone) from a
/// dropped request (shell saw it but answered nothing) — the silent-catch trap
/// this substrate exists to avoid.
// Caller-side: only `BridgeHandle::call` builds these, so dead in the binary
// until the live-bridge transport calls it.
#[allow(dead_code)]
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CallError {
    /// The shell is gone: the request channel is closed.
    #[error("bridge closed before the request could be delivered")]
    Closed,
    /// The shell received the request but dropped the reply channel unanswered.
    #[error("the shell dropped the request without replying")]
    Dropped,
    /// No reply arrived within the caller's bound.
    #[error("no reply within {0:?}")]
    Timeout(Duration),
}

/// One in-flight request: the payload plus the private channel its reply rides
/// back on.
type Envelope = (Request, oneshot::Sender<Reply>);

/// The caller side, held by a transport task. Cloneable so many transport tasks
/// can share one bridge into the shell.
// `tx`/`call` are exercised by tests and, in production, by the live-bridge
// transport wired next — dead in the binary until then.
#[allow(dead_code)]
#[derive(Clone)]
pub struct BridgeHandle {
    tx: mpsc::Sender<Envelope>,
}

#[allow(dead_code)]
impl BridgeHandle {
    /// Send `request` to the shell and await its reply, bounded by `timeout`.
    /// Never blocks past the bound: a shell that stalls yields
    /// [`CallError::Timeout`], not a hang.
    ///
    /// The bound covers *both* the enqueue and the wait — a full request channel
    /// (the shell wedged with `CAPACITY` requests already queued) would block the
    /// send indefinitely, so timing only the reply would leave that path
    /// unbounded, the very hang this exists to prevent.
    pub async fn call(&self, request: Request, timeout: Duration) -> Result<Reply, CallError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let round_trip = async {
            self.tx
                .send((request, reply_tx))
                .await
                .map_err(|_| CallError::Closed)?;
            // The shell dropped the reply channel without answering.
            reply_rx.await.map_err(|_| CallError::Dropped)
        };
        match tokio::time::timeout(timeout, round_trip).await {
            Ok(result) => result,
            // No progress within the bound — the caller fails fast, not hangs.
            Err(_) => Err(CallError::Timeout(timeout)),
        }
    }
}

/// The receiver side, drained by the iced subscription. Wrapped like
/// [`super::streams::PtyOutput`] so it is taken once into the stream and hashes
/// by identity, keeping the subscription stable across `view`/`update` cycles.
#[derive(Clone)]
pub struct Requests(Arc<Mutex<Option<mpsc::Receiver<Envelope>>>>);

impl Requests {
    fn new(rx: mpsc::Receiver<Envelope>) -> Self {
        Self(Arc::new(Mutex::new(Some(rx))))
    }
}

impl Hash for Requests {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.0) as usize).hash(state);
    }
}

/// The reply channel carried inside a [`Message::Bridge`]. `Message` must be
/// `Clone`, but a `oneshot::Sender` is not — so it lives behind a shared
/// take-once slot. Exactly one [`Self::answer`] sends; a duplicated message
/// finds the slot empty and the caller simply times out.
#[derive(Clone)]
pub struct ReplyPort(Arc<Mutex<Option<oneshot::Sender<Reply>>>>);

impl ReplyPort {
    fn new(tx: oneshot::Sender<Reply>) -> Self {
        Self(Arc::new(Mutex::new(Some(tx))))
    }

    /// Answer the caller, at most once. A missing receiver (caller already
    /// timed out and dropped its end) is not an error — the send just no-ops.
    pub fn answer(&self, reply: Reply) {
        if let Some(tx) = self.0.lock().ok().and_then(|mut slot| slot.take()) {
            // The caller may already have timed out and dropped its end; a
            // failed send is expected, not an error.
            let _ = tx.send(reply);
        }
    }
}

impl fmt::Debug for ReplyPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReplyPort")
    }
}

/// Build a bridge: the caller half for transport tasks, the receiver half for
/// the shell subscription.
pub fn channel() -> (BridgeHandle, Requests) {
    let (tx, rx) = mpsc::channel(CAPACITY);
    (BridgeHandle { tx }, Requests::new(rx))
}

/// Read the workspace snapshot from `core`. Pure — no mutation, no I/O — so the
/// shell can answer a `Snapshot` request straight from the state it already
/// owns.
pub fn snapshot(core: &App) -> Snapshot {
    let tabs = &core.workspace.tabs;
    Snapshot {
        tab_count: tabs.len(),
        // `active` is only meaningful when a tab exists; an empty workspace has
        // no active tab.
        active_tab: (!tabs.is_empty()).then_some(core.workspace.active),
    }
}

/// Answer one request from the state `core` already holds. The shell calls this
/// on the UI thread inside `update`, so it stays pure and cheap.
pub fn respond(core: &App, request: &Request) -> Reply {
    match request {
        Request::Snapshot => Reply::Snapshot(snapshot(core)),
    }
}

/// The iced subscription source: drains transport requests into
/// [`Message::Bridge`]s. Takes the receiver on first run; a duplicated
/// subscription (there is only ever one) idles rather than stealing requests.
pub(super) fn request_stream(source: &Requests) -> impl Stream<Item = Message> + use<> {
    let taken = source.0.lock().ok().and_then(|mut slot| slot.take());
    iced::stream::channel(
        CAPACITY,
        |mut out: iced::futures::channel::mpsc::Sender<Message>| async move {
            match taken {
                Some(mut rx) => {
                    while let Some((request, reply_tx)) = rx.recv().await {
                        let message = Message::Bridge {
                            request,
                            reply: ReplyPort::new(reply_tx),
                        };
                        let _ = out.send(message).await;
                    }
                }
                // No receiver (a duplicate sub): park forever rather than end,
                // matching the PTY-stream convention.
                None => std::future::pending().await,
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use termherd_core::{Event, Launch, LaunchSpec};

    /// Open `n` shell tabs in a fresh `App`, so a snapshot has real workspace
    /// state to read.
    fn app_with_tabs(n: usize) -> App {
        let mut app = App::new();
        for i in 0..n {
            app.apply(Event::LaunchSession(LaunchSpec {
                cwd: Some(format!("/tmp/p{i}")),
                launch: Launch::Shell,
                title: format!("tab {i}"),
            }));
        }
        app
    }

    #[test]
    fn snapshot_reads_tab_count_and_active_tab() {
        let app = app_with_tabs(3);
        let snap = snapshot(&app);
        assert_eq!(snap.tab_count, 3, "three tabs were opened");
        assert_eq!(
            snap.active_tab,
            Some(app.workspace.active),
            "active tab mirrors the workspace"
        );
    }

    #[test]
    fn snapshot_of_an_empty_workspace_has_no_active_tab() {
        let snap = snapshot(&App::new());
        assert_eq!(snap.tab_count, 0);
        assert_eq!(
            snap.active_tab, None,
            "an empty workspace has no active tab"
        );
    }

    /// The happy path: the shell answers, the caller gets the reply.
    #[tokio::test]
    async fn call_returns_the_reply_when_answered() {
        let (handle, requests) = channel();
        // Stand in for the shell subscription: drain one request, answer it.
        let shell = tokio::spawn(async move {
            let mut rx = requests.0.lock().expect("lock").take().expect("receiver");
            let (request, reply_tx) = rx.recv().await.expect("one request");
            assert_eq!(request, Request::Snapshot);
            let _ = reply_tx.send(Reply::Snapshot(Snapshot {
                tab_count: 2,
                active_tab: Some(1),
            }));
        });
        let reply = handle
            .call(Request::Snapshot, Duration::from_secs(1))
            .await
            .expect("a reply within the bound");
        assert_eq!(
            reply,
            Reply::Snapshot(Snapshot {
                tab_count: 2,
                active_tab: Some(1),
            })
        );
        shell.await.expect("shell task");
    }

    /// A shell that receives the request but never answers must not hang the
    /// caller — the timeout fires.
    #[tokio::test]
    async fn call_times_out_when_the_shell_never_answers() {
        let (handle, requests) = channel();
        // Hold the request (and its reply channel) without answering.
        let _shell = tokio::spawn(async move {
            let mut rx = requests.0.lock().expect("lock").take().expect("receiver");
            let held = rx.recv().await.expect("one request");
            std::future::pending::<()>().await;
            drop(held);
        });
        let err = handle
            .call(Request::Snapshot, Duration::from_millis(50))
            .await
            .expect_err("no answer means an error");
        assert_eq!(err, CallError::Timeout(Duration::from_millis(50)));
    }

    /// The shell saw the request but dropped its reply channel — distinct from a
    /// timeout: the caller learns immediately, no waiting out the bound.
    #[tokio::test]
    async fn call_errors_when_the_shell_drops_the_reply() {
        let (handle, requests) = channel();
        tokio::spawn(async move {
            let mut rx = requests.0.lock().expect("lock").take().expect("receiver");
            let (_request, reply_tx) = rx.recv().await.expect("one request");
            drop(reply_tx);
        });
        let err = handle
            .call(Request::Snapshot, Duration::from_secs(5))
            .await
            .expect_err("a dropped reply is an error");
        assert_eq!(err, CallError::Dropped);
    }

    /// No shell at all (its receiver dropped): the send fails as `Closed`, not
    /// as a timeout — the caller need not wait out the bound.
    #[tokio::test]
    async fn call_errors_when_the_bridge_is_closed() {
        let (handle, requests) = channel();
        drop(requests);
        let err = handle
            .call(Request::Snapshot, Duration::from_secs(5))
            .await
            .expect_err("no receiver is an error");
        assert_eq!(err, CallError::Closed);
    }

    /// A shell wedged with a full request channel must still bound the caller:
    /// the enqueue can't make progress, and the timeout has to cover that, not
    /// only the reply wait.
    #[tokio::test]
    async fn call_times_out_when_the_request_channel_is_full() {
        let (handle, requests) = channel();
        // Keep the receiver alive (so this is "full", not "closed") but never
        // drain it, then fill the channel to capacity.
        let mut queued = 0;
        loop {
            let (reply_tx, _reply_rx) = oneshot::channel();
            if handle.tx.try_send((Request::Snapshot, reply_tx)).is_err() {
                break;
            }
            queued += 1;
        }
        assert!(queued >= 1, "the channel accepted at least one request");
        let err = handle
            .call(Request::Snapshot, Duration::from_millis(50))
            .await
            .expect_err("a full channel still bounds the caller");
        assert_eq!(err, CallError::Timeout(Duration::from_millis(50)));
        drop(requests);
    }

    /// The receiver side is driven by iced's executor in production, not tokio —
    /// so draining a request into a [`Message::Bridge`] must work with no tokio
    /// runtime present. Poll the stream on a bare futures executor to prove it.
    #[test]
    fn request_stream_drains_a_request_without_a_tokio_runtime() {
        use iced::futures::StreamExt;
        let (handle, requests) = channel();
        let (reply_tx, _reply_rx) = oneshot::channel();
        handle
            .tx
            .try_send((Request::Snapshot, reply_tx))
            .expect("queue one request");
        // Close the sender so the stream ends after draining the one request.
        drop(handle);
        let mut stream = Box::pin(request_stream(&requests));
        let message = iced::futures::executor::block_on(stream.next()).expect("one message");
        match message {
            Message::Bridge { request, .. } => assert_eq!(request, Request::Snapshot),
            other => panic!("expected a bridge message, got {other:?}"),
        }
    }

    #[test]
    fn reply_port_answers_at_most_once() {
        let (tx, rx) = oneshot::channel();
        let port = ReplyPort::new(tx);
        let snap = Reply::Snapshot(Snapshot {
            tab_count: 0,
            active_tab: None,
        });
        port.answer(snap.clone());
        // A second answer (e.g. a duplicated message) is a no-op, not a panic.
        port.answer(snap.clone());
        assert_eq!(
            rx.blocking_recv(),
            Ok(snap),
            "the first answer reached the caller"
        );
    }
}
