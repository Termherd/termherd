//! Filterable workspace snapshot — the structured "DOM" an MCP client (or the
//! G1 dev loop) reads to perceive termherd before acting on it.
//!
//! The design constraint is context size: a driving agent must never pay for
//! state it did not ask for. So a snapshot is **light by default and filterable
//! at call** ([`SnapshotFilter`]) — the bare read is structure only (config,
//! sidebar, tabs), and terminal text is opt-in and scoped. Deep per-session
//! reads live in a later `read_terminal` rung; this is the overview, the map
//! you zoom from.
//!
//! Two parts of the picture are **adapter-injected** ([`SnapshotInputs`]): the
//! resolved config (settings live in the `app` adapter) and per-session terminal
//! text (the grid lives in the `pty` adapter). Everything else — the sidebar,
//! the tabs, the focus — the pure core reads from `App` itself. **Pure**: no
//! I/O, no clock, no panic.

use std::collections::BTreeMap;

use crate::app::SessionStatus;

/// Default number of trailing terminal lines a snapshot keeps per scoped
/// session — enough to see the current screen, small enough to stay cheap.
pub const DEFAULT_TEXT_LINES: usize = 40;

/// A top-level snapshot section. An absent section is *not built* (not merely
/// empty) — dropping sections is how a caller keeps the payload small.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Resolved settings summary (adapter-injected).
    Config,
    /// The session-browser sidebar: projects and filter state.
    Sidebar,
    /// The open tabs and their panes.
    Tabs,
}

/// Which sessions' terminal text to fold in. `None` (the default) keeps the
/// snapshot text-free — the cheap structural read. `Focused` adds only the
/// focused pane; `Only` scopes to named handles. Text is always truncated to
/// the filter's `text_lines`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalScope {
    /// No terminal text at all.
    None,
    /// Only the focused pane's text.
    Focused,
    /// Only these session handles' text. The resulting `terminals` map is keyed
    /// (and so ordered) by handle, not by the order requested here.
    Only(Vec<u64>),
}

/// How a caller shapes a snapshot. [`Default`] is light: all three structural
/// sections, no terminal text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotFilter {
    /// Sections to build. Absent = skipped.
    pub sections: Vec<Section>,
    /// Which sessions' terminal text to include.
    pub terminals: TerminalScope,
    /// Trailing lines kept per scoped terminal.
    pub text_lines: usize,
}

impl Default for SnapshotFilter {
    fn default() -> Self {
        Self {
            sections: vec![Section::Config, Section::Sidebar, Section::Tabs],
            terminals: TerminalScope::None,
            text_lines: DEFAULT_TEXT_LINES,
        }
    }
}

impl SnapshotFilter {
    /// Whether `section` was requested.
    #[must_use]
    pub fn includes(&self, section: Section) -> bool {
        self.sections.contains(&section)
    }
}

/// The parts of the picture the pure core cannot read itself, handed in by the
/// adapters that own them: the adapter-owned config bits and the per-session
/// terminal text (`handle -> full visible text`; the core truncates and scopes
/// it). Core never does the I/O that produces these.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SnapshotInputs {
    /// The config bits the `app` adapter owns (settings live there). `None` when
    /// the caller did not request config; the builder folds in the live font
    /// size it *can* read.
    pub config: Option<ConfigInput>,
    /// Full visible text per session handle, from the `pty` adapter. The core
    /// keeps only the scoped handles and truncates each to `text_lines`.
    pub terminals: BTreeMap<u64, String>,
}

/// The config bits the pure core cannot read — the terminal scheme, the record
/// budget, and how many keymap bindings the user overrode. Assembled by the
/// adapter that owns `settings.json`; the builder folds this into a
/// [`ConfigSummary`] together with the live font size it owns.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigInput {
    /// Active terminal colour scheme name, if one is set.
    pub terminal_scheme: Option<String>,
    /// Effective screencast frame rate (defaults applied when unset).
    pub record_fps: u32,
    /// Effective screencast scale factor (defaults applied when unset).
    pub record_scale: f32,
    /// How many keymap bindings the user overrode from the defaults.
    pub keymap_overrides: usize,
}

/// The resolved config a driving agent sees: the live font size (stamped by the
/// builder from the state `core` owns) plus the adapter-injected [`ConfigInput`]
/// bits.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigSummary {
    /// Effective terminal font size (base + zoom) — read live from `core`.
    pub font_size: f32,
    /// Active terminal colour scheme name, if one is set.
    pub terminal_scheme: Option<String>,
    /// Effective screencast frame rate (defaults applied when unset).
    pub record_fps: u32,
    /// Effective screencast scale factor (defaults applied when unset).
    pub record_scale: f32,
    /// How many keymap bindings the user overrode from the defaults.
    pub keymap_overrides: usize,
}

/// The whole snapshot. Structural sections are `Option` — present iff the filter
/// asked for them; [`Self::focus`] is always present (it is cheap and central).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceSnapshot {
    /// The active tab and focused session — always reported.
    pub focus: FocusRef,
    /// Resolved config, when [`Section::Config`] was requested and injected.
    pub config: Option<ConfigSummary>,
    /// The sidebar, when [`Section::Sidebar`] was requested.
    pub sidebar: Option<SidebarSnapshot>,
    /// The open tabs, when [`Section::Tabs`] was requested.
    pub tabs: Option<Vec<TabSnapshot>>,
    /// Scoped, truncated terminal text by handle. Empty unless the filter's
    /// [`TerminalScope`] asked for text.
    pub terminals: BTreeMap<u64, String>,
}

/// The cheap focus pointer: which tab is active and which session holds focus.
/// Both `None` on an empty workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FocusRef {
    /// Active tab index, or `None` when no tab is open.
    pub tab: Option<usize>,
    /// Focused session handle, or `None` when nothing is focused.
    pub session: Option<u64>,
}

/// The sidebar as a driving agent sees it: the filter knobs plus a light
/// per-project row (path, live-visible session count, fold state). The full
/// per-session browser rows are deliberately *not* here — that is a deeper read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarSnapshot {
    /// Whether the sidebar is collapsed to give the terminal full width.
    pub hidden: bool,
    /// Current search query (empty = no filtering).
    pub search: String,
    /// Whether search is restricted to titles.
    pub search_titles_only: bool,
    /// Whether archived sessions are shown.
    pub show_archived: bool,
    /// One row per visible project, in render order.
    pub projects: Vec<ProjectSnapshot>,
}

/// A light sidebar project row: its path, how many sessions it shows under the
/// current filter, and whether its list is folded shut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSnapshot {
    /// Real project path.
    pub path: String,
    /// Sessions visible under the current search/archive filter.
    pub session_count: usize,
    /// Whether the project's session list is folded shut.
    pub collapsed: bool,
}

/// One open tab: its label, the most-urgent status among its sessions, and its
/// panes left-to-right. `active` marks the tab currently shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabSnapshot {
    /// Whether this is the active tab.
    pub active: bool,
    /// The label the user sees.
    pub title: String,
    /// Most-urgent activity among the tab's sessions, or `None` if none live.
    pub status: Option<SessionStatus>,
    /// The tab's panes, in pane order (one for a plain tab, several for a split).
    pub panes: Vec<PaneSnapshot>,
}

/// One pane in a tab: the session it hosts, addressed by its stable handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneSnapshot {
    /// Stable external handle (the runtime session id).
    pub handle: u64,
    /// Whether it runs a shell or the Claude CLI.
    pub kind: SessionKind,
    /// Real project path the session runs in, if known.
    pub cwd: Option<String>,
    /// Current activity.
    pub status: SessionStatus,
}

/// The kind of program a session runs, as an external client sees it — the
/// core-side domain tag, derived from [`crate::Launch`] (which also carries the
/// resume id a plain tag should not expose).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Shell,
    Claude,
}

/// Keep only the last `lines` lines of `text` (all of it when it is shorter, an
/// empty string when `lines` is 0). Line breaks are normalised to `\n`.
#[must_use]
pub fn tail_lines(text: &str, lines: usize) -> String {
    if lines == 0 {
        return String::new();
    }
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn default_filter_is_light_all_sections_no_text() {
        let filter = SnapshotFilter::default();
        assert!(filter.includes(Section::Config));
        assert!(filter.includes(Section::Sidebar));
        assert!(filter.includes(Section::Tabs));
        assert_eq!(filter.terminals, TerminalScope::None);
        assert_eq!(filter.text_lines, DEFAULT_TEXT_LINES);
    }

    #[test]
    fn includes_reflects_the_requested_sections() {
        let filter = SnapshotFilter {
            sections: vec![Section::Tabs],
            ..SnapshotFilter::default()
        };
        assert!(filter.includes(Section::Tabs));
        assert!(!filter.includes(Section::Config));
        assert!(!filter.includes(Section::Sidebar));
    }

    #[test]
    fn tail_lines_keeps_only_the_last_n() {
        let text = "a\nb\nc\nd\ne";
        assert_eq!(tail_lines(text, 2), "d\ne");
        assert_eq!(tail_lines(text, 0), "");
        // Asking for more than exist returns all of them, unchanged.
        assert_eq!(tail_lines(text, 99), text);
    }

    proptest! {
        /// The tail is exactly the last `min(n, len)` of the text's own lines,
        /// re-joined. Asserted at the string level, not by re-splitting the tail:
        /// `str::lines()` treats a run of empty lines as collapsible, so a
        /// re-split would not roundtrip. `str::lines()` is also the normalisation
        /// `tail_lines` applies, so both sides use it as the source of truth.
        #[test]
        fn tail_lines_is_the_trailing_source_lines(
            fragments in prop::collection::vec("[a-z ]{0,8}", 0..40),
            n in 0usize..50,
        ) {
            let text = fragments.join("\n");
            let source: Vec<&str> = text.lines().collect();
            let want = source.len().min(n);
            prop_assert!(want <= n);
            let expected = source[source.len() - want..].join("\n");
            prop_assert_eq!(tail_lines(&text, n), expected);
        }
    }
}
