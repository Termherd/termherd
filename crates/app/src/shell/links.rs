//! `termherd://` links: the inbox watch, and the drain that turns each link
//! into a `core` event. A link is untrusted input, so it only reaches `core`
//! once `deeplink::parse` has read it whole.

use std::path::PathBuf;
use std::time::Duration;

use iced::Task;
use iced::futures::{SinkExt, Stream, StreamExt};
use termherd_core::Event;
use termherd_core::deeplink::{self, LinkRequest};

use super::{Message, Shell};
use crate::link_inbox;

const INBOX_DEBOUNCE: Duration = Duration::from_millis(100);

impl Shell {
    /// Act on every link waiting in the inbox, oldest first.
    pub(super) fn drain_link_inbox(&mut self) -> Task<Message> {
        let Some(dir) = self.data_dir.as_ref().map(link_inbox::inbox_dir) else {
            return Task::none();
        };
        let tasks: Vec<_> = link_inbox::drain(&dir)
            .into_iter()
            .filter_map(|url| match deeplink::parse(&url) {
                Ok(request) => {
                    let effects = self.core.apply(event_of(request));
                    Some(self.perform(effects))
                }
                Err(error) => {
                    tracing::warn!(%error, ?url, "termherd link refused");
                    None
                }
            })
            .collect();
        Task::batch(tasks)
    }
}

/// The `core` event a parsed link asks for, its repo read off the filesystem
/// as well as written.
fn event_of(request: LinkRequest) -> Event {
    match request {
        LinkRequest::OpenRepo(path) => {
            let real = resolved(&path);
            Event::OpenRepo {
                resolved: (real != path).then_some(real),
                path,
            }
        }
    }
}

/// `repo` as the filesystem resolves it, the form a shell reports its working
/// directory in: links followed, `..` gone. A path that does not resolve is
/// kept as written. Windows resolves to a verbatim `\\?\C:\…` path, which no
/// shell reports, so that prefix is dropped.
pub(crate) fn resolved(repo: &str) -> String {
    std::fs::canonicalize(repo)
        .ok()
        .and_then(|path| without_verbatim_prefix(&path.to_string_lossy()).map(str::to_owned))
        .unwrap_or_else(|| repo.to_owned())
}

/// `path` with a Windows verbatim drive prefix (`\\?\C:\…`) dropped. `None`
/// for a verbatim UNC path (`\\?\UNC\…`), which has no plain drive form.
fn without_verbatim_prefix(path: &str) -> Option<&str> {
    match path.strip_prefix(r"\\?\") {
        Some(rest) if rest.starts_with("UNC\\") => None,
        Some(rest) => Some(rest),
        None => Some(path),
    }
}

/// Watch the inbox and emit [`Message::LinkInboxChanged`] once at start, for
/// the links that arrived before the watch, then once per burst of entries.
/// If the watch cannot start, only the links waiting at launch are read.
// `&PathBuf` is imposed by `Subscription::run_with`, which passes `&D` to a
// plain fn pointer.
#[allow(clippy::ptr_arg)]
pub(super) fn inbox_watch_stream(dir: &PathBuf) -> impl Stream<Item = Message> + use<> {
    let dir = dir.clone();
    iced::stream::channel(
        4,
        |mut output: iced::futures::channel::mpsc::Sender<Message>| async move {
            let (tx, mut rx) = iced::futures::channel::mpsc::unbounded::<()>();
            let watch = std::fs::create_dir_all(&dir)
                .map_err(|error| error.to_string())
                .and_then(|()| {
                    termherd_scan::watch_changes(dir, INBOX_DEBOUNCE, move || {
                        let _ = tx.unbounded_send(());
                    })
                    .map_err(|error| error.to_string())
                });
            if output.send(Message::LinkInboxChanged).await.is_err() {
                return;
            }
            match watch {
                Ok(handle) => {
                    while rx.next().await.is_some() {
                        if output.send(Message::LinkInboxChanged).await.is_err() {
                            break;
                        }
                    }
                    drop(handle);
                }
                Err(error) => {
                    tracing::warn!(%error, "link inbox watch unavailable; links apply at restart");
                    iced::futures::future::pending::<()>().await;
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verbatim_drive_path_reads_as_a_plain_one() {
        assert_eq!(
            without_verbatim_prefix(r"\\?\C:\Users\me"),
            Some(r"C:\Users\me")
        );
        assert_eq!(
            without_verbatim_prefix("/private/tmp"),
            Some("/private/tmp")
        );
        assert_eq!(without_verbatim_prefix(r"\\?\UNC\server\share"), None);
    }
}
