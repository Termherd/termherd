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

/// A candidate that turned out to be a real file, in the two forms the two
/// decisions about it need.
///
/// They differ only when a symlink is involved, and that difference is the
/// whole point: `notes.md -> payload.app` is a document by name and a program
/// by nature, and the OS opener follows the link. Judging the name would let a
/// clone carrying one symlink walk straight past [`crate::paths::runs_on_open`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPath {
    /// What to hand the opener: lexically collapsed, symlinks left intact — so
    /// what opens is the path the user pointed at, and (on Windows) not a
    /// `\\?\` verbatim form the shell handler mishandles.
    pub path: PathBuf,
    /// The same file with every symlink followed, falling back to `path` when
    /// the filesystem could not say. **This is what policy judges.**
    pub real: PathBuf,
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
        session: SessionId,
        probe: Option<TargetProbe>,
    ) -> Vec<Effect> {
        let Some(probe) = probe else {
            // Only the session that *holds* the hover may clear it. Every
            // canvas in the tree sees every pointer event, so in a split the
            // pane the pointer just left reports "nothing here" in the same
            // batch as the pane it entered reports a target — and an
            // unconditional clear would delete, half the time, the hover that
            // was just set beside it.
            if self.hovers(session) {
                self.hover = None;
            }
            self.clear_pending_for(session);
            return Vec::new();
        };
        match probe.kind {
            ProbeKind::Url(url) => {
                self.clear_pending_for(session);
                self.hover = Some(TermHover {
                    session,
                    row: probe.row,
                    start: probe.start,
                    end: probe.end,
                    target: HoverTarget::Url(url),
                });
                Vec::new()
            }
            ProbeKind::Path {
                candidate,
                line,
                col,
            } => {
                let request = PathRequest {
                    session,
                    purpose: PathPurpose::Hover,
                    row: probe.row,
                    start: probe.start,
                    end: probe.end,
                    candidate,
                    line,
                    col,
                };
                // Already the outstanding question: re-asking would `stat` the
                // same path again on every pointer move within one span.
                if self.pending_path.as_ref() == Some(&request) {
                    return Vec::new();
                }
                self.hover = None;
                self.pending_path = Some(request.clone());
                vec![Effect::ResolvePath {
                    roots: self.path_roots(session),
                    request,
                }]
            }
        }
    }

    /// A resolution came back. Which of the two questions it answers decides
    /// everything: a hover underlines, a click opens, and a candidate that
    /// resolved nowhere does neither — silently, because the alternative is
    /// an error for every `and/or` on screen.
    pub(super) fn path_resolved(
        &mut self,
        request: &PathRequest,
        resolved: Option<ResolvedPath>,
    ) -> Vec<Effect> {
        // A program is not a document: the OS handler would run it, and a `ls`
        // of an untrusted clone is enough to put one on screen. Filtered here,
        // once, rather than at each of the two outcomes below — so a target
        // that cannot be opened cannot be underlined either, and the two can
        // never drift apart.
        //
        // Judged on `real`, never on `path`: a symlink named `notes.md` that
        // points at `payload.app` is a document by name and a program by
        // nature, and it is the nature the opener acts on. Whether the refusal
        // applies at all is `App::refuses_to_open`'s call — a configured editor
        // command consults no association, so it has nothing to protect.
        let path = resolved
            .filter(|r| !self.refuses_to_open(&r.real))
            .map(|r| r.path);
        match request.purpose {
            PathPurpose::Hover => {
                // The pointer may have moved on while the `stat` ran. Applying a
                // stale answer would underline a span the pointer has left.
                if self.pending_path.as_ref() != Some(request) {
                    return Vec::new();
                }
                self.pending_path = None;
                self.hover = path.map(|path| TermHover {
                    session: request.session,
                    row: request.row,
                    start: request.start,
                    end: request.end,
                    target: HoverTarget::Path {
                        path,
                        line: request.line,
                        col: request.col,
                    },
                });
                Vec::new()
            }
            PathPurpose::Open => path.map_or_else(Vec::new, |path| {
                vec![Effect::OpenPath(self.open_target(
                    path,
                    request.line,
                    request.col,
                ))]
            }),
        }
    }

    /// The user Ctrl/Cmd+clicked a target. A URL opens straight away; a path is
    /// re-resolved rather than read off the hover, so a click never depends on
    /// a hover having landed first.
    ///
    /// Activating consumes the hover either way: the OS handoff often steals
    /// focus, so no pointer event would arrive to clear the underline.
    pub(super) fn activate_target(
        &mut self,
        session: SessionId,
        probe: TargetProbe,
    ) -> Vec<Effect> {
        if self.hovers(session) {
            self.hover = None;
        }
        self.clear_pending_for(session);
        match probe.kind {
            ProbeKind::Url(url) => {
                let url = url.trim();
                if url.is_empty() {
                    Vec::new()
                } else {
                    vec![Effect::OpenUrl(url.to_owned())]
                }
            }
            ProbeKind::Path {
                candidate,
                line,
                col,
            } => vec![Effect::ResolvePath {
                roots: self.path_roots(session),
                request: PathRequest {
                    session,
                    purpose: PathPurpose::Open,
                    row: probe.row,
                    start: probe.start,
                    end: probe.end,
                    candidate,
                    line,
                    col,
                },
            }],
        }
    }

    /// The directories a session's relative paths could be relative to. An
    /// unknown session yields none rather than nothing to resolve against —
    /// the adapter treats an empty set as "only an absolute path can match".
    fn path_roots(&self, session: SessionId) -> PathRoots {
        self.sessions
            .get(&session)
            .map(|live| PathRoots {
                cwd: live.cwd.as_ref().map(PathBuf::from),
                launch_cwd: live.launch_cwd.as_ref().map(PathBuf::from),
            })
            .unwrap_or_default()
    }

    /// Whether the current hover belongs to `session`.
    fn hovers(&self, session: SessionId) -> bool {
        self.hover.as_ref().is_some_and(|h| h.session == session)
    }

    /// Drop the outstanding resolution **only if it is this session's**.
    ///
    /// One predicate rather than the condition written out at each of the three
    /// sites that clear it: every canvas in the tree sees every pointer event,
    /// so "whoever speaks last wins" is never the rule here — and an invariant
    /// spelled three times drifts on the first edit that touches one of them.
    fn clear_pending_for(&mut self, session: SessionId) {
        if self
            .pending_path
            .as_ref()
            .is_some_and(|pending| pending.session == session)
        {
            self.pending_path = None;
        }
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

    /// A resolution with no symlink in the way: the two forms coincide.
    fn plain(path: &str) -> ResolvedPath {
        ResolvedPath {
            path: path.into(),
            real: path.into(),
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
    fn one_panes_empty_hover_does_not_clear_another_panes() {
        // In a split, every canvas sees every pointer event: the pane the
        // pointer left reports "nothing here" in the same batch as the pane it
        // entered reports a target. An unconditional clear would delete the
        // hover that was just set beside it — in one of the two travel
        // directions, so the underline would work going one way and not back.
        let mut app = App::new();
        app.apply(Event::TermTarget {
            session: sid(1),
            probe: Some(url_probe()),
        });
        app.apply(Event::TermTarget {
            session: sid(2),
            probe: None,
        });
        assert!(
            app.term_hover().is_some(),
            "the other pane's empty report is not about this hover"
        );
        // The pane that owns it still clears it.
        app.apply(Event::TermTarget {
            session: sid(1),
            probe: None,
        });
        assert_eq!(app.term_hover(), None);
    }

    #[test]
    fn one_panes_empty_hover_does_not_cancel_another_panes_resolution() {
        // Same hazard one step earlier: dropping the pending request would
        // make the answer arrive stale and be discarded, so the underline
        // would never appear at all.
        let mut app = App::new();
        let effects = app.apply(Event::TermTarget {
            session: sid(1),
            probe: Some(path_probe()),
        });
        let (request, _) = resolve_request(&effects);
        let request = request.clone();
        app.apply(Event::TermTarget {
            session: sid(2),
            probe: None,
        });
        app.apply(Event::PathResolved {
            request,
            resolved: Some(plain("/repo/src/main.rs")),
        });
        assert!(
            app.term_hover().is_some(),
            "the resolution survived the sibling pane's empty report"
        );
    }

    #[test]
    fn no_branch_clears_another_sessions_pending_resolution() {
        // The session-scoped rule holds at *every* site that clears it, not
        // just at the empty-probe one. Two of the three used to clear
        // unconditionally: one invariant, three spellings, two of them wrong.
        // Derived from the branches themselves rather than a hand-listed set,
        // so a new way to clear cannot quietly opt out.
        /// One way the app clears state, named so the failure says which.
        type Clear = (&'static str, Box<dyn Fn(&mut App)>);
        let clears: [Clear; 3] = [
            (
                "an empty probe",
                Box::new(|app: &mut App| {
                    app.apply(Event::TermTarget {
                        session: sid(2),
                        probe: None,
                    });
                }),
            ),
            (
                "a URL settling",
                Box::new(|app: &mut App| {
                    app.apply(Event::TermTarget {
                        session: sid(2),
                        probe: Some(url_probe()),
                    });
                }),
            ),
            (
                "an activation",
                Box::new(|app: &mut App| {
                    app.apply(Event::ActivateTarget {
                        session: sid(2),
                        probe: url_probe(),
                    });
                }),
            ),
        ];
        for (what, act) in clears {
            let mut app = App::new();
            let effects = app.apply(Event::TermTarget {
                session: sid(1),
                probe: Some(path_probe()),
            });
            let (request, _) = resolve_request(&effects);
            let request = request.clone();
            act(&mut app);
            app.apply(Event::PathResolved {
                request,
                resolved: Some(plain("/repo/src/main.rs")),
            });
            assert!(
                app.term_hover().is_some_and(|h| h.session == sid(1)),
                "{what} in another pane must not cancel this resolution"
            );
        }
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
                    resolved: Some(plain("/repo/src/main.rs")),
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
                resolved: None
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
            resolved: Some(plain("/repo/src/main.rs")),
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
            resolved: Some(plain("/repo/src/main.rs")),
        });
        // No command configured: the OS handler, which cannot honour the `:42`
        // the terminal printed. The position is not lost on the way — it is
        // applied by `open_target`, and this handoff has nowhere to put it.
        assert!(matches!(
            opened.as_slice(),
            [Effect::OpenPath(crate::open::OpenTarget::SystemHandler { path })]
                if path == std::path::Path::new("/repo/src/main.rs")
        ));

        // Nothing on disk → nothing happens. A remote session prints paths
        // that exist only on the far side; a silent no-op beats an error
        // dialog per click.
        assert!(
            app.apply(Event::PathResolved {
                request,
                resolved: None
            })
            .is_empty()
        );
    }

    #[test]
    fn a_program_is_neither_underlined_nor_opened() {
        // One `ls` of an untrusted clone puts `payload.app` on screen. The OS
        // handler would run it, so it is not a link — and refusing it at the
        // hover as well as the click is what makes that visible before the
        // click rather than after.
        let probe = |candidate: &str| TargetProbe {
            row: 3,
            start: 0,
            end: 11,
            kind: ProbeKind::Path {
                candidate: candidate.into(),
                line: None,
                col: None,
            },
        };
        for program in ["payload.app", "report.command", "readme.EXE", "run.desktop"] {
            let mut app = App::new();
            let effects = app.apply(Event::TermTarget {
                session: sid(1),
                probe: Some(probe(program)),
            });
            let (request, _) = resolve_request(&effects);
            let request = request.clone();
            // The resolver found it: it really is on disk.
            app.apply(Event::PathResolved {
                request,
                resolved: Some(plain(&format!("/repo/{program}"))),
            });
            assert_eq!(app.term_hover(), None, "{program} must not underline");

            let opened = app.apply(Event::ActivateTarget {
                session: sid(1),
                probe: probe(program),
            });
            let (request, _) = resolve_request(&opened);
            let request = request.clone();
            assert!(
                app.apply(Event::PathResolved {
                    request,
                    resolved: Some(plain(&format!("/repo/{program}"))),
                })
                .is_empty(),
                "{program} must not open"
            );
        }
    }

    #[test]
    fn a_configured_command_opens_the_position_the_os_handoff_could_not() {
        let mut app = App::new();
        app.apply(Event::OpenCommandLoaded(Some(
            crate::open::OpenCommand::parse("code -g {path}:{line}:{col}").expect("valid command"),
        )));
        let effects = app.apply(Event::ActivateTarget {
            session: sid(1),
            probe: path_probe(),
        });
        let (request, _) = resolve_request(&effects);
        let request = request.clone();
        let opened = app.apply(Event::PathResolved {
            request,
            resolved: Some(plain("/repo/src/main.rs")),
        });
        match opened.as_slice() {
            [Effect::OpenPath(crate::open::OpenTarget::Editor { program, args })] => {
                assert_eq!(program, "code");
                assert_eq!(args, &["-g", "/repo/src/main.rs:42:1"]);
            }
            other => panic!("expected an editor open, got {other:?}"),
        }
    }

    #[test]
    fn a_configured_command_underlines_and_opens_what_the_association_would_run() {
        // The refusal exists because the OS handler runs a program instead of
        // showing it. An explicit command asks no association, so the same
        // `payload.app` — and the `.exe` the Windows table never let the
        // denylist cover — become ordinary files again, underline included.
        for program in ["payload.app", "build.exe"] {
            let probe = TargetProbe {
                row: 3,
                start: 0,
                end: 11,
                kind: ProbeKind::Path {
                    candidate: program.into(),
                    line: None,
                    col: None,
                },
            };
            let mut app = App::new();
            app.apply(Event::OpenCommandLoaded(Some(
                crate::open::OpenCommand::parse("code {path}").expect("valid command"),
            )));

            let effects = app.apply(Event::TermTarget {
                session: sid(1),
                probe: Some(probe.clone()),
            });
            let (request, _) = resolve_request(&effects);
            let request = request.clone();
            app.apply(Event::PathResolved {
                request,
                resolved: Some(plain(&format!("/repo/{program}"))),
            });
            assert!(
                app.term_hover().is_some(),
                "{program} is a file the editor can show"
            );

            let opened = app.apply(Event::ActivateTarget {
                session: sid(1),
                probe,
            });
            let (request, _) = resolve_request(&opened);
            let request = request.clone();
            let opened = app.apply(Event::PathResolved {
                request,
                resolved: Some(plain(&format!("/repo/{program}"))),
            });
            match opened.as_slice() {
                [Effect::OpenPath(crate::open::OpenTarget::Editor { args, .. })] => {
                    assert_eq!(args, &[format!("/repo/{program}")]);
                }
                other => panic!("expected {program} to open in the editor, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_symlink_is_judged_by_what_it_points_at_not_by_its_name() {
        // The counterpart of the adapter's symlink test, on this side of the
        // port: given a document name and a program behind it, the refusal must
        // follow the program. Reading `path` here instead of `real` is exactly
        // the bug, and it would look correct.
        let probe = TargetProbe {
            row: 3,
            start: 0,
            end: 8,
            kind: ProbeKind::Path {
                candidate: "notes.md".into(),
                line: None,
                col: None,
            },
        };
        let disguised = ResolvedPath {
            path: "/repo/notes.md".into(),
            real: "/repo/payload.app".into(),
        };

        let mut app = App::new();
        let effects = app.apply(Event::TermTarget {
            session: sid(1),
            probe: Some(probe.clone()),
        });
        let (request, _) = resolve_request(&effects);
        let request = request.clone();
        app.apply(Event::PathResolved {
            request,
            resolved: Some(disguised.clone()),
        });
        assert_eq!(
            app.term_hover(),
            None,
            "a document name over a program is not underlined"
        );

        let opened = app.apply(Event::ActivateTarget {
            session: sid(1),
            probe,
        });
        let (request, _) = resolve_request(&opened);
        let request = request.clone();
        assert!(
            app.apply(Event::PathResolved {
                request,
                resolved: Some(disguised),
            })
            .is_empty(),
            "and it does not open either"
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
        let session = match app
            .apply(Event::LaunchSession(LaunchSpec {
                cwd: Some("/proj".into()),
                launch: Launch::Shell,
                title: "work".into(),
            }))
            .as_slice()
        {
            [Effect::Spawn(spec)] => spec.session,
            other => panic!("expected Spawn, got {other:?}"),
        };
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
        assert_eq!(
            roots.launch_cwd.as_deref(),
            Some(std::path::Path::new("/proj")),
            "the launch directory survives a cd"
        );
    }

    #[test]
    fn an_unknown_session_yields_no_roots_rather_than_nothing_at_all() {
        // A probe can outlive its session (a tab closed under the pointer).
        // Empty roots mean "only an absolute path can match" — not a panic.
        let mut app = App::new();
        let effects = app.apply(Event::TermTarget {
            session: sid(99),
            probe: Some(path_probe()),
        });
        let (_, roots) = resolve_request(&effects);
        assert_eq!(roots, &PathRoots::default());
    }
}
