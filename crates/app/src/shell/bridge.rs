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
    workspace::SplitDir,
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
    /// A workspace mutation, for the orchestration MCP tools. Unlike the two
    /// read requests it is answered off `respond` — the shell applies it through
    /// `App::apply` and performs the effects, since it needs `&mut` state.
    Act(Action),
    /// Park until a session's activity reaches one of `targets`, for the
    /// `wait_for_status` MCP tool. The odd one out on this bridge: its reply
    /// lands in a *later* `update` — whichever one carries the status change —
    /// so the shell holds the reply port meanwhile. Answers at once when the
    /// session already sits on a target, or when the handle is unknown.
    WaitForStatus {
        session: u64,
        targets: Vec<SessionStatus>,
    },
    /// The visible text of one session's terminal, for the `read_terminal` MCP
    /// tool — the deep read the light `Snapshot` deliberately leaves out.
    ReadTerminal { session: u64, lines: usize },
}

/// The result of a [`Request::WaitForStatus`]. `error` is `Some` only when the
/// handle named no live session, in which case nothing was awaited. Otherwise
/// `status` is the activity the session had reached when the wait settled.
///
/// A caller that gives up first sees [`CallError::Timeout`] instead — the wait
/// is bounded by the caller's own clock, never by one the shell keeps (Q7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitOutcome {
    /// The status that settled the wait.
    pub status: Option<SessionStatus>,
    /// Why nothing was awaited, or `None` when the wait ran.
    pub error: Option<String>,
}

/// The result of a [`Request::ReadTerminal`]. The three cases stay distinct: an
/// unknown handle (`error`), a live session whose screen has not rendered yet
/// (`text: None`), and real text — a scoped `Snapshot` collapses the first two
/// into one absent key, which an agent cannot act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRead {
    /// The visible text, trailing lines only, or `None` when nothing has
    /// rendered yet.
    pub text: Option<String>,
    /// Why nothing was read, or `None` when the read ran.
    pub error: Option<String>,
}

/// A workspace mutation an MCP client asks termherd to perform. Each variant
/// maps onto one or more existing core [`Event`](termherd_core::Event)s applied
/// through `App::apply` — the orchestration surface never bypasses the state
/// machine, it drives termherd exactly as a keystroke does. A `session`/`pane`
/// field is the stable handle as [`SessionInfo::handle`] reports it, resolved to
/// a live `SessionId` before anything is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Open a new session in `project` (or the home dir when `None`), running
    /// `kind`. → `Event::LaunchSession`, via the shell's own launch path.
    Open {
        project: Option<String>,
        kind: SessionKind,
    },
    /// Split a pane, opening a fresh session beside it. Splits the focused pane,
    /// or `pane` when given (revealed first, so a pane in another tab is
    /// reachable). → `[RevealPane +] SplitFocused`.
    Split { pane: Option<u64>, dir: SplitDir },
    /// Bring the pane hosting `session` into view, activating its tab when it
    /// lives in another one. → `Event::RevealPane`.
    Focus { session: u64 },
    /// Give the tab at `tab` a manual name (blank reverts to the derived title,
    /// core's rule). → `Event::RenameTab`.
    Rename { tab: usize, title: String },
    /// Close a pane — the focused one, or `pane` when given (revealed first). A
    /// lone pane closes its whole tab (core collapses to `close_tab`, killing the
    /// PTY). → `[RevealPane +] CloseFocusedPane`.
    Close { pane: Option<u64> },
    /// Type `bytes` into a session's PTY without waiting; a caller that needs
    /// to synchronise follows with [`Request::WaitForStatus`].
    /// → `Event::TerminalInput`.
    Run { session: u64, bytes: Vec<u8> },
}

/// The result of an [`Action`]. `error` is `Some` only when the action was
/// rejected before touching state — an unknown handle or an out-of-range tab —
/// in which case nothing was applied. Otherwise `focused` reports the session
/// handle that holds focus once the action settled (the new pane after
/// open/split, the target after focus), so an agent gets act→observe in one
/// round trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionOutcome {
    /// The resulting focused session handle, or `None` when nothing is focused.
    pub focused: Option<String>,
    /// Why the action was rejected, or `None` when it applied.
    pub error: Option<String>,
}

impl ActionOutcome {
    /// A rejection that touched no state — an unresolved handle / index.
    pub fn rejected(reason: impl Into<String>) -> Self {
        Self {
            focused: None,
            error: Some(reason.into()),
        }
    }

    /// An applied action, reporting the resulting focused handle.
    pub fn applied(focused: Option<String>) -> Self {
        Self {
            focused,
            error: None,
        }
    }
}

/// The app's answer to a [`Request`].
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Snapshot(WorkspaceSnapshot),
    Sessions(Vec<SessionInfo>),
    Acted(ActionOutcome),
    Waited(WaitOutcome),
    Terminal(TerminalRead),
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
    pub(super) fn new(tx: oneshot::Sender<Reply>) -> Self {
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

impl ReplyPort {
    /// Whether the caller is gone — it timed out and dropped its end, so no
    /// answer can land. Lets the shell sweep parked waiters instead of holding
    /// them for a session that may never reach its target.
    pub(super) fn abandoned(&self) -> bool {
        self.0
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(oneshot::Sender::is_closed))
            // A taken slot means it was already answered; treat it as gone too.
            .unwrap_or(true)
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
        // Actions mutate, so they can't answer off a `&App`; the shell branches
        // them to `perform_action` before reaching here. This arm is the
        // defensive default should that routing ever be bypassed.
        Request::Act(_) => Reply::Acted(ActionOutcome::rejected(
            "an action reached the read-only responder; the shell must apply it",
        )),
        // A wait parks and a terminal read needs the `pty` adapter's screens —
        // neither is answerable from a bare `&App`, so the shell serves them
        // itself. These arms are the defensive default, as above.
        Request::WaitForStatus { .. } => Reply::Waited(WaitOutcome {
            status: None,
            error: Some("a wait reached the read-only responder; the shell must park it".into()),
        }),
        Request::ReadTerminal { .. } => Reply::Terminal(TerminalRead {
            text: None,
            error: Some("a terminal read reached the read-only responder".into()),
        }),
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
    spawn_test_shell_seq(requests, vec![Some(reply)]).map_into_first()
}

/// A test shell that serves a *sequence* of requests, answering the nth with
/// `replies[n]` — `None` meaning "receive it and stay silent", which is how a
/// caller-side timeout is exercised. Returns the requests in arrival order.
#[cfg(test)]
pub(crate) fn spawn_test_shell_seq(
    requests: Requests,
    replies: Vec<Option<Reply>>,
) -> tokio::task::JoinHandle<Vec<Request>> {
    tokio::spawn(async move {
        let mut rx = requests.take().expect("a receiver on first take");
        let mut seen = Vec::new();
        // Un-answered ports are kept, not dropped: dropping one raises
        // `Dropped` at the caller, and it is the *timeout* path we exercise.
        let mut held = Vec::new();
        for reply in replies {
            let Some((request, reply_tx)) = rx.recv().await else {
                break;
            };
            seen.push(request);
            match reply {
                Some(reply) => {
                    let _ = reply_tx.send(reply);
                }
                None => held.push(reply_tx),
            }
        }
        seen
    })
}

/// Adapter so the single-reply helper keeps its one-request return shape.
#[cfg(test)]
trait FirstRequest {
    fn map_into_first(self) -> tokio::task::JoinHandle<Request>;
}

#[cfg(test)]
impl FirstRequest for tokio::task::JoinHandle<Vec<Request>> {
    fn map_into_first(self) -> tokio::task::JoinHandle<Request> {
        tokio::spawn(async move {
            let mut seen = self.await.expect("test shell task");
            assert_eq!(seen.len(), 1, "the single-reply shell served one request");
            seen.remove(0)
        })
    }
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
