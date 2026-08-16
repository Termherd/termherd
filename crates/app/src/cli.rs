//! The command line in front of the window.
//!
//! `termherd` is a GUI binary and stays one: every invocation opens the
//! workspace, except the handful that ask it a question instead. Today there is
//! one such question — `termherd mcp-config`, which prints the `mcpServers`
//! snippet for the *running* instance's live bridge.
//!
//! That subcommand must be recognised **before** the single-instance lock is
//! taken and before any window opens: it exists to report on an instance that is
//! already running, and a launcher that tripped that instance's lock (exit 2) or
//! opened a second window while asking would be answering a different question
//! from the one asked.

use crate::attach::{self, AttachError};

/// What this invocation was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    /// Open the workspace — every invocation that is not a question.
    Gui,
    /// Print the `mcpServers` snippet for the running instance, then exit.
    McpConfig,
}

/// The subcommand that asks the running instance for its bridge config.
const MCP_CONFIG: &str = "mcp-config";

/// Read the command out of `args`, which is the raw `std::env::args` including
/// the program name.
///
/// Anything unrecognised is [`Command::Gui`], not an error: the binary is
/// launched by Finder, by a `.desktop` entry and by macOS with arguments it
/// never asked for, and refusing to start over one of those would be the
/// "double-click does nothing" bug in a new costume.
pub(crate) fn parse<I: IntoIterator<Item = String>>(args: I) -> Command {
    match args.into_iter().nth(1).as_deref() {
        Some(MCP_CONFIG) => Command::McpConfig,
        _ => Command::Gui,
    }
}

/// Answer `termherd mcp-config`: read what the running instance published,
/// prove it is live, and render the snippet.
pub(crate) fn mcp_config() -> Result<String, AttachError> {
    let path = attach::bridge_path().ok_or(AttachError::NotPublished)?;
    mcp_config_at(&path)
}

/// The three steps, over an explicit path so the composition itself is testable
/// — the home directory is the one part of it that cannot be pointed elsewhere.
///
/// Order matters: read, *then* prove, then render. Rendering a config from a
/// file that has outlived its instance would fail on the caller's first tool
/// call instead of here, where the failure can still say what to do about it.
fn mcp_config_at(path: &std::path::Path) -> Result<String, AttachError> {
    let published = attach::read(path)?;
    attach::probe(&published)?;
    Ok(attach::mcp_servers(&published))
}

/// What to tell the user when there is no config to print. Each case names the
/// thing to *do* about it, since the four failures call for four different
/// responses and "failed to get config" would send them all to the same place.
pub(crate) fn explain(error: &AttachError) -> String {
    match error {
        AttachError::NotPublished => format!(
            "no running termherd is publishing its live bridge. \
             Set mcp.attach to true in settings.json (or launch with {}=1) and restart it.",
            attach::ATTACH_ENV
        ),
        AttachError::Unreadable => {
            "the published bridge file could not be read. Restart termherd to rewrite it."
                .to_owned()
        }
        AttachError::Stale => {
            "the published bridge is no longer answering: the instance that wrote it has exited. \
             Restart termherd."
                .to_owned()
        }
        AttachError::Unauthorized => {
            "the published bridge refused its own token. Restart termherd to publish a fresh one."
                .to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        std::iter::once("termherd")
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn the_subcommand_is_recognised_and_nothing_else_is() {
        assert_eq!(parse(argv(&[MCP_CONFIG])), Command::McpConfig);
        assert_eq!(parse(argv(&[])), Command::Gui);
    }

    /// macOS hands a Finder launch `-psn_0_12345`, and a `.desktop` entry can
    /// carry anything. An unrecognised argument opens the workspace; it must
    /// never refuse to start.
    #[test]
    fn an_unrecognised_argument_still_opens_the_workspace() {
        for stray in ["-psn_0_123456", "--verbose", "some/file.txt"] {
            assert_eq!(
                parse(argv(&[stray])),
                Command::Gui,
                "{stray} must not refuse to start"
            );
        }
    }

    /// The subcommand is the first token, not any token: a path that happens to
    /// be named `mcp-config` opened from Finder is not a subcommand.
    #[test]
    fn only_the_first_argument_selects_the_subcommand() {
        assert_eq!(parse(argv(&["--theme", MCP_CONFIG])), Command::Gui);
    }

    /// The composed answer, end to end: a real listener, a real published file,
    /// a real handshake. The three steps are tested apart in `attach`; this
    /// pins that they are wired in the right order, which is the one thing
    /// their individual tests cannot see.
    #[tokio::test]
    async fn a_published_live_endpoint_yields_the_snippet_for_it() {
        let (bridge, _requests) = crate::shell::bridge::channel();
        let tokens = crate::mcp::Tokens::default();
        let endpoint = crate::mcp::serve(bridge, tokens.clone())
            .await
            .expect("bind the loopback server");
        let published = attach::Published {
            url: endpoint.url.clone(),
            token: tokens.issue(),
        };
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("bridge.json");
        attach::publish(&path, &published).expect("publish");

        let snippet = tokio::task::spawn_blocking(move || mcp_config_at(&path))
            .await
            .expect("join")
            .expect("a config for a live endpoint");
        assert_eq!(snippet, attach::mcp_servers(&published));
    }

    /// The file is read before the endpoint is dialled, so an absent one is
    /// reported as such rather than as a connection that failed.
    #[test]
    fn no_published_file_is_reported_before_anything_is_dialled() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(
            mcp_config_at(&dir.path().join("bridge.json")),
            Err(AttachError::NotPublished)
        );
    }

    /// Four failures, four things to do about them. A shared message would send
    /// a user whose instance is running but whose token is stale looking for a
    /// process that is not the problem.
    #[test]
    fn every_failure_explains_what_to_do_about_that_failure() {
        let messages: Vec<String> = [
            AttachError::NotPublished,
            AttachError::Unreadable,
            AttachError::Stale,
            AttachError::Unauthorized,
        ]
        .iter()
        .map(explain)
        .collect();

        for message in &messages {
            assert!(!message.is_empty(), "every failure says something");
        }
        let distinct: std::collections::HashSet<&String> = messages.iter().collect();
        assert_eq!(
            distinct.len(),
            messages.len(),
            "no two failures share a message: {messages:?}"
        );
        assert!(
            messages[0].contains("mcp.attach"),
            "an unpublished bridge points at the setting that publishes it, got: {}",
            messages[0]
        );
        assert!(
            messages[2].contains("restart") || messages[2].contains("no longer"),
            "a stale file says the instance is gone, got: {}",
            messages[2]
        );
    }
}
