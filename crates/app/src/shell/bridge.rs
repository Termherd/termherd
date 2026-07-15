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
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::futures::{SinkExt, Stream};
use termherd_core::{
    App, Launch, LiveSession, SessionStatus, SnapshotFilter, SnapshotInputs, WorkspaceSnapshot,
};
use tokio::sync::{mpsc, oneshot};

use super::Message;
use super::streams::TakeOnceSource;

/// Depth of the transport→shell request channel. Bounded so a burst of requests
/// applies backpressure to the caller instead of growing memory without limit —
/// a wedged shell fills this and `BridgeHandle::call` then times out.
const REQUEST_CHANNEL_CAPACITY: usize = 32;

/// Buffer of the iced subscription stream that carries drained requests on to
/// `update`. Independent of the transport channel depth above; sized only to
/// smooth bursts between one `recv` and the next `view`/`update` cycle.
const MESSAGE_STREAM_BUFFER: usize = 32;

/// What an external transport can ask the running app: a read-only, filterable
/// workspace snapshot, or the live-session list. Both answer straight from the
/// state the shell already owns.
// Caller-side substrate: the `Request`/`BridgeHandle::call` half is driven by
// tests and by the MCP tools in production; it reads as dead in the binary only
// where a variant is not yet built by a tool. The receiver half (`respond`,
// `request_stream`) is live via the subscription and `update`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// A filterable, read-only snapshot of the workspace.
    Snapshot(SnapshotFilter),
    /// Every live session, for the `list_sessions` MCP tool.
    ListSessions,
}

/// The app's answer to a [`Request`].
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Snapshot(WorkspaceSnapshot),
    Sessions(Vec<SessionInfo>),
}

/// The kind of program a session runs, as an MCP client sees it. Distinct from
/// `core::Launch` (which also carries the resume id) so the external surface
/// stays a plain tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Shell,
    Claude,
}

/// One live session as an external MCP client sees it.
///
/// `handle` is the **stable external id** — the runtime `SessionId`, minted once
/// at launch (`Sessions::allocate`) and never re-keyed — deliberately distinct
/// from `resume_id`, the Claude session id that *does* re-key on a fork /
/// plan-accept (Q6). An MCP client addresses `handle`; it outlives the re-key
/// that `resume_id` would not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    /// The stable external handle: the runtime session id as a decimal string.
    pub handle: String,
    /// The tab label hosting this session.
    pub title: String,
    /// Real project path the session runs in, if known.
    pub cwd: Option<String>,
    /// Whether it runs a shell or the Claude CLI.
    pub kind: SessionKind,
    /// The Claude session id this launch resumes, if any — the *unstable* id
    /// (see the type note); `None` for a shell or a fresh Claude session.
    pub resume_id: Option<String>,
    /// Current activity (FR8).
    pub status: SessionStatus,
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
    /// (the shell wedged with `REQUEST_CHANNEL_CAPACITY` requests already queued)
    /// blocks the send indefinitely, so timing only the reply would leave that
    /// path unbounded, the very hang this exists to prevent.
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

/// The receiver side, drained by the iced subscription: the shared take-once
/// source over the transport request channel.
pub type Requests = TakeOnceSource<mpsc::Receiver<Envelope>>;

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
    let (tx, rx) = mpsc::channel(REQUEST_CHANNEL_CAPACITY);
    (BridgeHandle { tx }, Requests::new(rx))
}

/// Every live session as a stable-handle [`SessionInfo`] list, sorted by handle
/// so the external surface is deterministic. Pure read of the registry `core`
/// already owns.
pub fn list_sessions(core: &App) -> Vec<SessionInfo> {
    let mut live: Vec<&LiveSession> = core.sessions.values().collect();
    // Deterministic ascending-handle order — the registry map is unordered, and
    // an external API must not shuffle its rows between calls.
    live.sort_by_key(|s| s.id.0);
    live.into_iter()
        .map(|s| SessionInfo {
            handle: s.id.0.get().to_string(),
            title: core
                .workspace
                .tab_of(s.id)
                .and_then(|index| core.workspace.tabs.get(index))
                .map(|tab| tab.title.clone())
                .unwrap_or_default(),
            cwd: s.cwd.clone(),
            kind: match s.launch {
                Launch::Shell => SessionKind::Shell,
                Launch::Claude { .. } => SessionKind::Claude,
            },
            resume_id: s.launch.resume_id().map(str::to_owned),
            status: s.status,
        })
        .collect()
}

/// Answer one request from the state `core` already holds, plus the
/// adapter-owned `inputs` (config + terminal text) the shell gathered for a
/// snapshot. The shell calls this on the UI thread inside `update`, so it stays
/// pure and cheap; `inputs` is empty for requests that need no injection.
pub fn respond(core: &App, request: &Request, inputs: &SnapshotInputs) -> Reply {
    match request {
        Request::Snapshot(filter) => Reply::Snapshot(core.snapshot(filter, inputs)),
        Request::ListSessions => Reply::Sessions(list_sessions(core)),
    }
}

/// The iced subscription source: drains transport requests into
/// [`Message::Bridge`]s. Takes the receiver on first run; a duplicated
/// subscription (there is only ever one) idles rather than stealing requests.
pub(super) fn request_stream(source: &Requests) -> impl Stream<Item = Message> + use<> {
    let taken = source.take();
    iced::stream::channel(
        MESSAGE_STREAM_BUFFER,
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

/// Test double for the shell side of the bridge: take the receiver, answer the
/// next single request with `reply`, and return the request that arrived.
/// Lets other modules' tests (e.g. the MCP tools) exercise a real round-trip
/// without standing up the iced shell. `take` is `pub(super)`, so this helper —
/// which lives inside the module — is how a sibling module reaches it.
#[cfg(test)]
pub(crate) fn spawn_test_shell(
    requests: Requests,
    reply: Reply,
) -> tokio::task::JoinHandle<Request> {
    tokio::spawn(async move {
        let mut rx = requests.take().expect("a receiver on first take");
        let (request, reply_tx) = rx.recv().await.expect("one request");
        let _ = reply_tx.send(reply);
        request
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use termherd_core::{Event, Launch, LaunchSpec, SessionStatus, SnapshotFilter, SnapshotInputs};

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

    /// Launch one Claude session (fresh or resumed) in `app`, returning its
    /// handle string.
    fn launch_claude(app: &mut App, cwd: &str, title: &str, resume: Option<&str>) -> String {
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some(cwd.to_owned()),
            launch: Launch::Claude {
                resume: resume.map(str::to_owned),
            },
            title: title.to_owned(),
        }));
        let id = app.workspace.focused_session().expect("a focused session");
        id.0.get().to_string()
    }

    #[test]
    fn list_sessions_is_empty_for_a_fresh_app() {
        assert!(list_sessions(&App::new()).is_empty());
    }

    #[test]
    fn list_sessions_reports_each_live_session_sorted_by_handle() {
        let app = app_with_tabs(3);
        let sessions = list_sessions(&app);
        assert_eq!(sessions.len(), 3, "three sessions were launched");
        let handles: Vec<&str> = sessions.iter().map(|s| s.handle.as_str()).collect();
        assert_eq!(
            handles,
            ["1", "2", "3"],
            "handles are the runtime ids, ascending"
        );
        // Each carries the tab title and cwd it was launched with.
        assert_eq!(sessions[0].title, "tab 0");
        assert_eq!(sessions[0].cwd.as_deref(), Some("/tmp/p0"));
    }

    #[test]
    fn a_shell_session_has_kind_shell_and_no_resume_id() {
        let app = app_with_tabs(1);
        let info = &list_sessions(&app)[0];
        assert_eq!(info.kind, SessionKind::Shell);
        assert_eq!(info.resume_id, None);
    }

    #[test]
    fn a_sessions_handle_is_its_runtime_id_not_the_claude_resume_id() {
        let mut app = App::new();
        let handle = launch_claude(&mut app, "/proj", "proj", Some("claude-abc-123"));
        let info = &list_sessions(&app)[0];
        assert_eq!(info.kind, SessionKind::Claude);
        assert_eq!(info.handle, handle, "the handle is the runtime id");
        assert_eq!(
            info.resume_id.as_deref(),
            Some("claude-abc-123"),
            "the resume id is the Claude session id, reported separately"
        );
        assert_ne!(
            info.handle, "claude-abc-123",
            "the stable handle is never the Claude id (Q6)"
        );
    }

    #[test]
    fn the_external_handle_is_stable_across_a_status_change() {
        // The runtime id is minted once and never re-keyed, so the mutable part
        // of a session (its status, and — on a real re-key — its Claude id)
        // changing must never move the handle an MCP client addresses (Q6).
        let mut app = App::new();
        launch_claude(&mut app, "/proj", "proj", Some("claude-abc-123"));
        let before = list_sessions(&app)[0].handle.clone();
        let id = app.workspace.focused_session().expect("a focused session");
        app.apply(Event::StatusChanged {
            session: id,
            status: SessionStatus::Busy,
        });
        let after = &list_sessions(&app)[0];
        assert_eq!(after.handle, before, "the handle survives a status change");
        assert_eq!(after.status, SessionStatus::Busy, "but the status updates");
    }

    #[test]
    fn respond_answers_list_sessions_with_the_same_list() {
        let app = app_with_tabs(2);
        assert_eq!(
            respond(&app, &Request::ListSessions, &SnapshotInputs::default()),
            Reply::Sessions(list_sessions(&app)),
            "respond forwards the live-session list unchanged"
        );
    }

    #[test]
    fn respond_answers_snapshot_with_the_core_snapshot() {
        let app = app_with_tabs(2);
        let filter = SnapshotFilter::default();
        let inputs = SnapshotInputs::default();
        assert_eq!(
            respond(&app, &Request::Snapshot(filter.clone()), &inputs),
            Reply::Snapshot(app.snapshot(&filter, &inputs)),
            "respond forwards the core snapshot unchanged"
        );
    }

    /// The happy path: the shell answers, the caller gets the reply.
    #[tokio::test]
    async fn call_returns_the_reply_when_answered() {
        let (handle, requests) = channel();
        // Stand in for the shell subscription: drain one request, answer it.
        let shell = tokio::spawn(async move {
            let mut rx = requests.take().expect("receiver");
            let (request, reply_tx) = rx.recv().await.expect("one request");
            assert_eq!(request, Request::ListSessions);
            let _ = reply_tx.send(Reply::Sessions(Vec::new()));
        });
        let reply = handle
            .call(Request::ListSessions, Duration::from_secs(1))
            .await
            .expect("a reply within the bound");
        assert_eq!(reply, Reply::Sessions(Vec::new()));
        shell.await.expect("shell task");
    }

    /// A shell that receives the request but never answers must not hang the
    /// caller — the timeout fires.
    #[tokio::test]
    async fn call_times_out_when_the_shell_never_answers() {
        let (handle, requests) = channel();
        // Hold the request (and its reply channel) without answering.
        let _shell = tokio::spawn(async move {
            let mut rx = requests.take().expect("receiver");
            let held = rx.recv().await.expect("one request");
            std::future::pending::<()>().await;
            drop(held);
        });
        let err = handle
            .call(Request::ListSessions, Duration::from_millis(50))
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
            let mut rx = requests.take().expect("receiver");
            let (_request, reply_tx) = rx.recv().await.expect("one request");
            drop(reply_tx);
        });
        let err = handle
            .call(Request::ListSessions, Duration::from_secs(5))
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
            .call(Request::ListSessions, Duration::from_secs(5))
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
            if handle
                .tx
                .try_send((Request::ListSessions, reply_tx))
                .is_err()
            {
                break;
            }
            queued += 1;
        }
        assert!(queued >= 1, "the channel accepted at least one request");
        let err = handle
            .call(Request::ListSessions, Duration::from_millis(50))
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
            .try_send((Request::ListSessions, reply_tx))
            .expect("queue one request");
        // Close the sender so the stream ends after draining the one request.
        drop(handle);
        let mut stream = Box::pin(request_stream(&requests));
        let message = iced::futures::executor::block_on(stream.next()).expect("one message");
        match message {
            Message::Bridge { request, .. } => assert_eq!(request, Request::ListSessions),
            other => panic!("expected a bridge message, got {other:?}"),
        }
    }

    #[test]
    fn reply_port_answers_at_most_once() {
        let (tx, rx) = oneshot::channel();
        let port = ReplyPort::new(tx);
        let snap = Reply::Sessions(Vec::new());
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
