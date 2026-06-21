//! All user-facing UI strings, in one place (#60).
//!
//! English is the canonical UI language. Centralising every literal here means a
//! future i18n layer is "swap this catalogue", not "find every string": no
//! user-facing literal should live in the view/shell code. Static labels are
//! `const`s; strings built from runtime values are functions.

use termherd_core::SessionStatus;

// --- Sidebar ---
pub const SEARCH_PLACEHOLDER: &str = "Search…";
pub const TITLES_ONLY: &str = "Titles only";
pub const SHOW_ARCHIVED: &str = "Show archived";
pub const NO_SESSIONS: &str = "No sessions found.";
pub const NO_RESULTS: &str = "No results.";
pub const PLANS_AND_MEMORY: &str = "Plans & memory";
pub const RENAME_PLACEHOLDER: &str = "title…";
pub const SIDEBAR_LAUNCH_SHELL: &str = "Open a shell here";
pub const SIDEBAR_LAUNCH_CLAUDE: &str = "Start a fresh Claude session";

/// Sidebar message when a project scan fails.
#[must_use]
pub fn scan_failed(error: &str) -> String {
    format!("Scan failed: {error}")
}

// --- Welcome pane (no session open) ---
pub const WELCOME_HINT_OPEN: &str = "Use $ for a shell or 🤖 for Claude beside a project,";
pub const WELCOME_HINT_RESUME: &str = "or click a session to resume it.";

/// The "N session(s) in M project(s)" summary on the welcome pane.
#[must_use]
pub fn welcome_counts(sessions: usize, projects: usize) -> String {
    format!("{sessions} session(s) in {projects} project(s)")
}

// --- Doc viewer ---
pub const DOC_CLOSE: &str = "✕ close";
pub const DOC_SAVE: &str = "💾 save";
pub const DOC_SAVED: &str = "saved";
pub const DOC_MODIFIED: &str = "• modified";

/// Shown in the doc pane when a plan/memory file can't be read.
#[must_use]
pub fn doc_read_failed(error: impl std::fmt::Display) -> String {
    format!("(could not read: {error})")
}

// --- Session hover card ---
/// The card's meta line: relative last activity (if known) + message count.
#[must_use]
pub fn session_meta(age: Option<&str>, count: u32) -> String {
    match age {
        Some("now") => format!("Just now  ·  {count} messages"),
        Some(age) => format!("{age} ago  ·  {count} messages"),
        None => format!("{count} messages"),
    }
}

// --- Confirmations ---
pub const CANCEL: &str = "Cancel";
pub const CLOSE: &str = "Close";
pub const ARCHIVE: &str = "Archive";
pub const QUIT: &str = "Quit";

/// Close-a-tab confirmation prompt (#9).
#[must_use]
pub fn close_tab_prompt(title: &str) -> String {
    format!("Close “{title}”? The session will be terminated.")
}

/// Archive-a-session confirmation prompt (#20).
#[must_use]
pub fn archive_prompt(title: &str) -> String {
    format!("Archive “{title}”?")
}

/// Quit confirmation when live sessions would be hard-killed.
#[must_use]
pub fn quit_prompt(live: usize) -> String {
    format!(
        "Quit TermHerd? {live} active session(s) will be force-stopped — Claude will be killed."
    )
}

// --- Activity status (FR8) ---
/// The short label for a session's activity status.
#[must_use]
pub fn status_label(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Starting => "starting",
        SessionStatus::Busy => "busy",
        SessionStatus::Idle => "ready",
        SessionStatus::Attention => "attention",
        SessionStatus::Exited => "exited",
    }
}
