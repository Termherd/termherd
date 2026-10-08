//! All user-facing UI strings, in one place.
//!
//! English is the canonical UI language. Centralising every literal here means a
//! future i18n layer is "swap this catalogue", not "find every string": no
//! user-facing literal should live in the view/shell code. Static labels are
//! `const`s; strings built from runtime values are functions.

// --- Sidebar ---
pub const SEARCH_PLACEHOLDER: &str = "Search…";
pub const TITLES_ONLY: &str = "Titles only";
pub const SHOW_ARCHIVED: &str = "Show archived";
pub const NO_SESSIONS: &str = "No sessions found.";
pub const NO_RESULTS: &str = "No results.";
pub const PLANS_AND_MEMORY: &str = "Plans & memory";
pub const FAVORITES: &str = "★ Favorites";
pub const RENAME_PLACEHOLDER: &str = "title…";
pub const SIDEBAR_LAUNCH_SHELL: &str = "Open a shell here";
pub const SIDEBAR_LAUNCH_CLAUDE: &str = "Start a fresh Claude session";
pub const SIDEBAR_SHOW_LESS: &str = "show less";
pub const SIDEBAR_HIDE: &str = "◀ Hide";
pub const SIDEBAR_ADD_REPO: &str = "+ Add a repo";
pub const SIDEBAR_ADD_REPO_HINT: &str = "Pick a folder, or drop one on the window";
pub const SIDEBAR_FORGET_REPO: &str = "Remove this repo from the sidebar";
pub const SIDEBAR_SETTINGS: &str = "⚙";
pub const SIDEBAR_SETTINGS_HINT: &str = "Settings";
pub const SIDEBAR_REPO_NO_SESSIONS: &str = "No sessions yet — start one with ❯ or ✳";

/// Expander under a truncated session list: how many more are folded.
#[must_use]
pub fn sidebar_more(hidden: usize) -> String {
    format!("… {hidden} more")
}

/// Sidebar message when a project scan fails.
#[must_use]
pub fn scan_failed(error: &str) -> String {
    format!("Scan failed: {error}")
}

// --- Welcome pane (no session open) ---
pub const WELCOME_HINT_OPEN: &str = "Use ❯ for a shell or ✳ for Claude beside a project,";
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

/// The card line naming the Claude in a tab by the peer name other Claude
/// sessions address it by.
#[must_use]
pub fn agent_name(name: &str) -> String {
    format!("Agent: {name}")
}

// --- Tab context menu ---
pub const TAB_MENU_RENAME: &str = "Rename tab";
pub const TAB_MENU_COPY_AGENT_NAME: &str = "Copy agent name";
pub const TAB_MENU_NEW_SHELL: &str = "New shell here";
pub const TAB_MENU_NEW_CLAUDE: &str = "New Claude session here";
pub const TAB_MENU_SPLIT_RIGHT: &str = "Split right";
pub const TAB_MENU_SPLIT_DOWN: &str = "Split down";
pub const TAB_MENU_CLOSE: &str = "Close pane";
/// The card line naming the colour `/color` gave a session — the cue that
/// does not depend on telling the colours apart.
#[must_use]
pub fn session_color(name: &str) -> String {
    format!("Colour: {name}")
}

// --- Confirmations ---
pub const CANCEL: &str = "Cancel";
pub const CLOSE: &str = "Close";
pub const ARCHIVE: &str = "Archive";
pub const QUIT: &str = "Quit";
pub const SEND: &str = "Send";

/// The same prompt after a confirmation typed nothing: the line, and why.
#[must_use]
pub fn claude_command_refused(line: &str, reason: &str) -> String {
    format!("“{line}” was not typed: {reason}.")
}

/// Confirmation before termherd types a slash command into a Claude session.
/// Quotes the exact line, so what the user approves is what is typed.
#[must_use]
pub fn claude_command_prompt(line: &str, title: &str) -> String {
    if title.is_empty() {
        format!("Type “{line}” into Claude?")
    } else {
        format!("Type “{line}” into “{title}”?")
    }
}

/// Close-a-tab confirmation prompt.
#[must_use]
pub fn close_tab_prompt(title: &str) -> String {
    format!("Close “{title}”? The session will be terminated.")
}

/// Archive-a-session confirmation prompt.
#[must_use]
pub fn archive_prompt(title: &str) -> String {
    format!("Archive “{title}”?")
}

/// Quit confirmation. With open sessions it names how many will be hard-killed
/// (every non-exited session dies on quit, Claude or shell); with none (an
/// `alwaysConfirm` quit of an idle app) it's a plain confirm.
#[must_use]
pub fn quit_prompt(live: usize) -> String {
    if live == 0 {
        "Quit TermHerd?".to_string()
    } else {
        format!(
            "Quit TermHerd? {live} open session(s) will be force-stopped — any running work is lost."
        )
    }
}

/// Sidebar notice for a config file that existed and could not be used.
#[must_use]
pub fn config_unusable(file: &str, kept_as: Option<&std::path::Path>) -> String {
    match kept_as {
        Some(kept) => format!(
            "{file} could not be read; defaults are in use. Your file was kept as {}.",
            kept.display()
        ),
        None => format!("{file} could not be read; defaults are in use."),
    }
}
pub const DISMISS: &str = "✕";

// --- Settings panel ---
pub const SETTINGS_TITLE: &str = "Settings";
pub const SETTINGS_THEME: &str = "Interface theme";
pub const SETTINGS_SCHEME: &str = "Terminal colours";
pub const SETTINGS_SCHEME_BUILTIN: &str = "Built-in";
pub const SETTINGS_CLOSE: &str = "Close";
pub const SETTINGS_SAVED_NOTE: &str = "Changes apply now and are saved to settings.json.";
pub const SETTINGS_CLAUDE_NOTE: &str =
    "A running Claude session keeps its own light or dark look until it restarts.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_line_carries_the_name_itself() {
        // The card's line is the only place a human reads the name, so a label
        // that dropped it would leave a bare "Agent:" nothing else catches.
        assert!(agent_name("termherd-b0").ends_with("termherd-b0"));
    }
}
