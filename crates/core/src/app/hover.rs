//! What the pointer is hovering in a terminal grid with the link modifier
//! (Ctrl/Cmd) held: the span to underline, and what a click would open.
//!
//! The span is *found* in the shell — only it holds the grid — but the answer
//! lives here, so exactly one place decides what is underlined. That matters
//! the moment a target's validity is not a pure function of the row text: a
//! file path has to be checked against the filesystem, which is an adapter's
//! job and comes back as an [`Event`], and an event can only land in `core`.
//!
//! So a URL and a path are the same shape but not the same timing. A URL is
//! settled the instant it is seen. A path is a *candidate* until
//! [`ports::PathResolver`](crate::ports::PathResolver) says otherwise, and
//! until then nothing is underlined — an underline that later turns out to
//! point at nothing is worse than one that arrives a frame late.

use std::path::PathBuf;

use super::*;

/// A clickable target under the pointer: which cells to underline, and what
/// activating it means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermHover {
    /// The session whose grid the span belongs to. The canvas widget is reused
    /// across tabs, so a hover carries its owner rather than being implicitly
    /// the focused one.
    pub session: SessionId,
    /// The grid row, in viewport coordinates.
    pub row: u16,
    /// Column span `[start, end)` — the terminal stores one char per cell, so
    /// these are also character indices into the row's text.
    pub start: u16,
    pub end: u16,
    /// What a Ctrl/Cmd+click on the span would open.
    pub target: HoverTarget,
}

/// What activating a hovered span opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoverTarget {
    /// A URL, handed to the OS default handler.
    Url(String),
    /// A file that was checked and exists, with the position the terminal
    /// printed beside it.
    Path {
        path: PathBuf,
        line: Option<u32>,
        col: Option<u32>,
    },
}

/// What the canvas found under the pointer, before anyone has vouched for it.
/// The shell can name the cells and read the text; it cannot know whether a
/// path-shaped run is a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetProbe {
    pub row: u16,
    pub start: u16,
    pub end: u16,
    pub kind: ProbeKind,
}

/// The two natures of clickable text, told apart by whether they need checking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeKind {
    /// Settled on sight: a scheme is all a URL needs.
    Url(String),
    /// Path-shaped text that means nothing until the filesystem agrees.
    Path {
        candidate: String,
        line: Option<u32>,
        col: Option<u32>,
    },
}

/// Why a path is being resolved — the same question asked for two outcomes, so
/// the answer knows which one it owes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathPurpose {
    /// The pointer is resting on it: underline it if it exists.
    Hover,
    /// The user clicked it: open it if it exists.
    Open,
}

/// One outstanding resolution, echoed back with its answer so `core` can tell
/// which question was answered without minting correlation ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathRequest {
    pub session: SessionId,
    pub purpose: PathPurpose,
    pub row: u16,
    pub start: u16,
    pub end: u16,
    pub candidate: String,
    pub line: Option<u32>,
    pub col: Option<u32>,
}

/// The directories a relative candidate could be relative *to*, most specific
/// first. `core` names them; the adapter walks and stats them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathRoots {
    /// Where the session is now — the live `cwd`, which follows every `cd`.
    pub cwd: Option<PathBuf>,
    /// Where the session started. Kept because `cwd` moves: after `cd /tmp` it
    /// is the only remaining trace of the project the session belongs to.
    pub launch_cwd: Option<PathBuf>,
}

impl App {
    /// Record what the canvas found under the pointer, or [`None`] when it left
    /// every target.
    ///
    /// A URL settles here. A path candidate does not: the hover is cleared and
    /// a resolution asked for, so nothing is underlined until the filesystem
    /// has spoken.
    pub(super) fn set_term_target(
        &mut self,
        _session: SessionId,
        _probe: Option<TargetProbe>,
    ) -> Vec<Effect> {
        Vec::new()
    }

    /// A resolution came back. Which of the two questions it answers decides
    /// everything: a hover underlines, a click opens, and a candidate that
    /// resolved nowhere does neither — silently, because the alternative is
    /// an error for every `and/or` on screen.
    pub(super) fn path_resolved(
        &mut self,
        _request: &PathRequest,
        _path: Option<PathBuf>,
    ) -> Vec<Effect> {
        Vec::new()
    }

    /// The user Ctrl/Cmd+clicked a target. A URL opens straight away; a path is
    /// re-resolved rather than read off the hover, so a click never depends on
    /// a hover having landed first.
    ///
    /// Activating consumes the hover either way: the OS handoff often steals
    /// focus, so no pointer event would arrive to clear the underline.
    pub(super) fn activate_target(
        &mut self,
        _session: SessionId,
        _probe: TargetProbe,
    ) -> Vec<Effect> {
        Vec::new()
    }

    /// The clickable target under the pointer, if any.
    #[must_use]
    pub const fn term_hover(&self) -> Option<&TermHover> {
        self.hover.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testsupport::*;

    fn url_probe() -> TargetProbe {
        TargetProbe {
            row: 3,
            start: 4,
            end: 17,
            kind: ProbeKind::Url("https://ex.io".into()),
        }
    }

    fn path_probe() -> TargetProbe {
        TargetProbe {
            row: 3,
            start: 0,
            end: 15,
            kind: ProbeKind::Path {
                candidate: "src/main.rs".into(),
                line: Some(42),
                col: None,
            },
        }
    }

    /// The single `ResolvePath` in `effects`, or a failed test.
    fn resolve_request(effects: &[Effect]) -> (&PathRequest, &PathRoots) {
        match effects {
            [Effect::ResolvePath { request, roots }] => (request, roots),
            other => panic!("expected exactly one ResolvePath, got {other:?}"),
        }
    }

    #[test]
    fn a_url_settles_on_sight_with_no_resolution() {
        let mut app = App::new();
        let effects = app.apply(Event::TermTarget {
            session: sid(1),
            probe: Some(url_probe()),
        });
        assert!(effects.is_empty(), "a URL needs nobody's permission");
        assert_eq!(
            app.term_hover().map(|h| h.target.clone()),
            Some(HoverTarget::Url("https://ex.io".into()))
        );
    }

    #[test]
    fn leaving_every_target_clears_the_hover() {
        let mut app = App::new();
        app.apply(Event::TermTarget {
            session: sid(1),
            probe: Some(url_probe()),
        });
        assert!(
            app.apply(Event::TermTarget {
                session: sid(1),
                probe: None,
            })
            .is_empty()
        );
        assert_eq!(app.term_hover(), None);
    }

    #[test]
    fn a_path_candidate_underlines_nothing_until_it_resolves() {
        // The whole point of the async port: an underline that later turns out
        // to point at nothing is worse than one that arrives a frame late.
        let mut app = App::new();
        let effects = app.apply(Event::TermTarget {
            session: sid(1),
            probe: Some(path_probe()),
        });
        assert_eq!(app.term_hover(), None, "nothing is underlined yet");
        let (request, _) = resolve_request(&effects);
        assert_eq!(request.purpose, PathPurpose::Hover);
        assert_eq!(request.candidate, "src/main.rs");
        assert_eq!(request.line, Some(42));
    }

    #[test]
    fn a_resolved_path_underlines_and_an_unresolved_one_does_not() {
        let mut app = App::new();
        let effects = app.apply(Event::TermTarget {
            session: sid(1),
            probe: Some(path_probe()),
        });
        let (request, _) = resolve_request(&effects);
        let request = request.clone();

        let mut resolved = App::new();
        let effects = resolved.apply(Event::TermTarget {
            session: sid(1),
            probe: Some(path_probe()),
        });
        let (pending, _) = resolve_request(&effects);
        let pending = pending.clone();
        assert!(
            resolved
                .apply(Event::PathResolved {
                    request: pending,
                    path: Some("/repo/src/main.rs".into()),
                })
                .is_empty()
        );
        assert_eq!(
            resolved.term_hover().map(|h| h.target.clone()),
            Some(HoverTarget::Path {
                path: "/repo/src/main.rs".into(),
                line: Some(42),
                col: None,
            })
        );

        // The same probe that resolves nowhere — prose, or a path on the far
        // side of an ssh session — underlines nothing and says nothing.
        assert!(
            app.apply(Event::PathResolved {
                request,
                path: None
            })
            .is_empty()
        );
        assert_eq!(app.term_hover(), None);
    }

    #[test]
    fn a_stale_resolution_is_dropped_rather_than_applied() {
        // The pointer moved on while the stat ran. Applying the old answer
        // would underline a span the pointer has left.
        let mut app = App::new();
        let effects = app.apply(Event::TermTarget {
            session: sid(1),
            probe: Some(path_probe()),
        });
        let (stale, _) = resolve_request(&effects);
        let stale = stale.clone();
        app.apply(Event::TermTarget {
            session: sid(1),
            probe: None,
        });
        app.apply(Event::PathResolved {
            request: stale,
            path: Some("/repo/src/main.rs".into()),
        });
        assert_eq!(app.term_hover(), None);
    }

    #[test]
    fn resting_on_one_candidate_asks_only_once() {
        // Pointer moves within a span must not each cost a filesystem probe.
        let mut app = App::new();
        assert_eq!(
            app.apply(Event::TermTarget {
                session: sid(1),
                probe: Some(path_probe()),
            })
            .len(),
            1
        );
        assert!(
            app.apply(Event::TermTarget {
                session: sid(1),
                probe: Some(path_probe()),
            })
            .is_empty(),
            "the same candidate is not re-stat'd"
        );
    }

    #[test]
    fn clicking_a_url_opens_it_and_consumes_the_hover() {
        let mut app = App::new();
        app.apply(Event::TermTarget {
            session: sid(1),
            probe: Some(url_probe()),
        });
        // Read off a grid, a URL can pick up padding cells; the handler is what
        // trims, so nothing shells out on whitespace.
        let effects = app.apply(Event::ActivateTarget {
            session: sid(1),
            probe: TargetProbe {
                kind: ProbeKind::Url("  https://ex.io  ".into()),
                ..url_probe()
            },
        });
        assert!(matches!(
            effects.as_slice(),
            [Effect::OpenUrl(u)] if u == "https://ex.io"
        ));
        assert_eq!(app.term_hover(), None, "the click consumes the hover");
    }

    #[test]
    fn clicking_a_path_resolves_it_again_rather_than_trusting_the_hover() {
        // A click never depends on a hover having landed first — one extra
        // round trip, one state dependency fewer.
        let mut app = App::new();
        let effects = app.apply(Event::ActivateTarget {
            session: sid(1),
            probe: path_probe(),
        });
        let (request, _) = resolve_request(&effects);
        assert_eq!(request.purpose, PathPurpose::Open);
        assert_eq!(request.candidate, "src/main.rs");
    }

    #[test]
    fn an_opened_path_carries_its_position_and_an_unresolved_one_opens_nothing() {
        let mut app = App::new();
        let effects = app.apply(Event::ActivateTarget {
            session: sid(1),
            probe: path_probe(),
        });
        let (request, _) = resolve_request(&effects);
        let request = request.clone();
        let opened = app.apply(Event::PathResolved {
            request: request.clone(),
            path: Some("/repo/src/main.rs".into()),
        });
        assert!(matches!(
            opened.as_slice(),
            [Effect::OpenPath { path, line: Some(42), col: None }]
                if path == std::path::Path::new("/repo/src/main.rs")
        ));

        // Nothing on disk → nothing happens. A remote session prints paths
        // that exist only on the far side; a silent no-op beats an error
        // dialog per click.
        assert!(
            app.apply(Event::PathResolved {
                request,
                path: None
            })
            .is_empty()
        );
    }

    #[test]
    fn a_blank_url_opens_nothing() {
        let mut app = App::new();
        let effects = app.apply(Event::ActivateTarget {
            session: sid(1),
            probe: TargetProbe {
                kind: ProbeKind::Url("   ".into()),
                ..url_probe()
            },
        });
        assert!(effects.is_empty());
    }

    #[test]
    fn the_roots_are_the_sessions_live_and_launch_directories() {
        // `cwd` follows every `cd`, so it alone loses the project the session
        // belongs to; the launch directory is the only remaining trace.
        let mut app = App::new();
        let session = launch(&mut app, "work");
        app.apply(Event::SessionCwdChanged {
            session,
            cwd: "/proj/crates/pty".into(),
        });
        let effects = app.apply(Event::TermTarget {
            session,
            probe: Some(path_probe()),
        });
        let (_, roots) = resolve_request(&effects);
        assert_eq!(
            roots.cwd.as_deref(),
            Some(std::path::Path::new("/proj/crates/pty"))
        );
        assert!(
            roots.launch_cwd.is_some(),
            "the launch directory survives a cd"
        );
    }
}
