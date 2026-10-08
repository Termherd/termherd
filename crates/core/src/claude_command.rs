//! The slash commands termherd may type into a Claude session on the user's
//! behalf — a closed catalogue, never free text.
//!
//! Claude Code owns the information termherd shows about a session (its name,
//! its colour); when termherd wants to change one, it asks Claude by typing the
//! command a user would. Everything that decides *what* gets typed lives here,
//! pure: the catalogue, how an argument is made safe to type, and the exact
//! bytes. *Whether* it may be typed now (the session is a Claude, and idle) is
//! [`App::claude_command_check`](crate::App::claude_command_check).

use std::fmt;

pub use termherd_claude::color::ClaudeColor;

/// The longest argument typed, in characters. A name is a label, not a
/// paragraph; past this the input is more likely a stray paste than a name.
pub const MAX_ARGUMENT_CHARS: usize = 80;

/// Ctrl+U: Claude Code's prompt deletes from the cursor to the start of the
/// line. Sent first so a half-typed draft is not prefixed to the command.
///
/// It clears the line the cursor is on, which is the whole draft when the draft
/// is one line and the cursor sits at its end — the usual case. Escape and
/// Ctrl+C were rejected: on an empty prompt a double Escape opens Claude's
/// rewind menu and a double Ctrl+C exits it, so either could act on a state the
/// user left a moment ago.
pub const CLEAR_DRAFT: &[u8] = b"\x15";

/// Enter, as a terminal sends it. Written on its own, after the line, so the
/// prompt receives it as a keypress rather than as part of a burst of text.
pub const SUBMIT: &[u8] = b"\r";

/// One command from the catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeCommand {
    /// `/rename <name>` — the session's name.
    Rename(CommandArgument),
    /// `/color <colour>` — the session's colour.
    Color(ClaudeColor),
    /// `/desktop` — hand the session to the Claude desktop app.
    Desktop,
}

/// Why a command could not be built from what the caller gave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    /// Nothing typeable was left once the argument was made safe.
    #[error("the argument is empty once control characters are removed")]
    EmptyArgument,
}

/// A free-text argument made safe to type at Claude's prompt. The field is
/// private, so the only way to hold one is through [`Self::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandArgument(String);

impl CommandArgument {
    /// Make `raw` safe to type as the tail of a slash command, or refuse it
    /// when nothing would be left.
    ///
    /// The argument shares one line with the command and the Enter that
    /// submits it, so anything that could end that line early, or turn the
    /// Enter into something else, is taken out:
    ///
    /// - control characters (newline, tab, escape, the C1 range) become a
    ///   space — a newline would submit half the line, an escape would start a
    ///   key sequence;
    /// - line and paragraph separators become a space, for the same reason;
    /// - invisible formatting characters (zero-width, bidi overrides) are
    ///   dropped, so the line the confirmation shows is the line typed;
    /// - runs of whitespace collapse to one space, and the ends are trimmed;
    /// - a trailing backslash goes, since `\` then Enter is how Claude's prompt
    ///   inserts a newline instead of submitting;
    /// - the result is cut to [`MAX_ARGUMENT_CHARS`].
    ///
    /// # Errors
    ///
    /// [`CommandError::EmptyArgument`] when nothing typeable remains.
    pub fn new(raw: &str) -> Result<Self, CommandError> {
        let spaced: String = raw
            .chars()
            .filter(|c| !is_invisible_format(*c))
            .map(|c| if breaks_line(c) { ' ' } else { c })
            .collect();
        let collapsed = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
        let cut: String = collapsed.chars().take(MAX_ARGUMENT_CHARS).collect();
        let clean = cut.trim_end_matches(|c: char| c == '\\' || c.is_whitespace());
        if clean.is_empty() {
            return Err(CommandError::EmptyArgument);
        }
        Ok(Self(clean.to_owned()))
    }

    /// The argument as it will be typed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether `c` ends a line, or starts a key sequence, at a terminal prompt.
fn breaks_line(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')
}

/// Whether `c` renders as nothing while still changing how the text reads —
/// what would let the confirmation show one line and type another.
fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}'
    )
}

impl ClaudeCommand {
    /// `/rename <name>`, with `name` made safe to type.
    ///
    /// # Errors
    ///
    /// [`CommandError::EmptyArgument`] for a name with nothing typeable in it.
    pub fn rename(name: &str) -> Result<Self, CommandError> {
        CommandArgument::new(name).map(Self::Rename)
    }

    /// The line typed at Claude's prompt — what the confirmation shows, and,
    /// byte for byte, what [`Self::keystrokes`] sends.
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Self::Rename(name) => format!("/rename {}", name.as_str()),
            Self::Color(color) => format!("/color {}", color.name()),
            Self::Desktop => "/desktop".to_owned(),
        }
    }

    /// The writes that type this command: the draft cleared and the line
    /// typed, then Enter on its own.
    #[must_use]
    pub fn keystrokes(&self) -> [Vec<u8>; 2] {
        let mut typed = CLEAR_DRAFT.to_vec();
        typed.extend_from_slice(self.line().as_bytes());
        [typed, SUBMIT.to_vec()]
    }
}

impl fmt::Display for ClaudeCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.line())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn renamed(raw: &str) -> String {
        ClaudeCommand::rename(raw).expect("a typeable name").line()
    }

    #[test]
    fn each_command_renders_the_line_claude_expects() {
        assert_eq!(renamed("api work"), "/rename api work");
        assert_eq!(
            ClaudeCommand::Color(ClaudeColor::Cyan).line(),
            "/color cyan"
        );
        assert_eq!(
            ClaudeCommand::Color(ClaudeColor::Default).line(),
            "/color default"
        );
        assert_eq!(ClaudeCommand::Desktop.line(), "/desktop");
    }

    #[test]
    fn a_newline_in_a_name_cannot_submit_half_of_it() {
        assert_eq!(renamed("first\nsecond"), "/rename first second");
        assert_eq!(renamed("first\r\nsecond"), "/rename first second");
        assert_eq!(renamed("a\u{2028}b\u{2029}c"), "/rename a b c");
    }

    #[test]
    fn an_escape_sequence_in_a_name_is_defused() {
        // ESC and the C1 introducer both go, so no key sequence survives —
        // the printable tail is left as harmless text.
        assert_eq!(renamed("x\u{1b}[200~y"), "/rename x [200~y");
        assert_eq!(renamed("x\u{9b}2Jy"), "/rename x 2Jy");
        assert_eq!(renamed("tab\there\u{7f}"), "/rename tab here");
    }

    #[test]
    fn invisible_formatting_is_dropped_so_the_shown_line_is_the_typed_one() {
        assert_eq!(renamed("ab\u{202E}cd"), "/rename abcd");
        assert_eq!(renamed("a\u{200B}b\u{FEFF}"), "/rename ab");
    }

    #[test]
    fn whitespace_collapses_and_a_trailing_backslash_goes() {
        assert_eq!(renamed("  spaced    out  "), "/rename spaced out");
        assert_eq!(renamed("name\\"), "/rename name");
        assert_eq!(renamed("name \\\\ "), "/rename name");
        assert_eq!(renamed("name \\ \\"), "/rename name", "nor behind a space");
        assert_eq!(renamed("a\\b"), "/rename a\\b", "an inner one is text");
    }

    #[test]
    fn a_name_with_nothing_typeable_is_refused() {
        for raw in ["", "   ", "\n\t\r", "\u{1b}", "\\", " \\ ", "\u{200B}"] {
            assert_eq!(
                ClaudeCommand::rename(raw),
                Err(CommandError::EmptyArgument),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn a_long_name_is_cut_on_a_character_not_a_byte() {
        let long = "é".repeat(MAX_ARGUMENT_CHARS + 10);
        let line = renamed(&long);
        assert_eq!(
            line.trim_start_matches("/rename ").chars().count(),
            MAX_ARGUMENT_CHARS
        );
    }

    #[test]
    fn the_keystrokes_clear_the_draft_type_the_line_then_press_enter_apart() {
        let command = ClaudeCommand::Color(ClaudeColor::Red);
        let [typed, enter] = command.keystrokes();
        assert_eq!(typed, b"\x15/color red");
        assert_eq!(enter, b"\r");
    }

    proptest! {
        #[test]
        fn whatever_the_name_the_typed_line_is_one_safe_line(raw in any::<String>()) {
            let Ok(command) = ClaudeCommand::rename(&raw) else {
                return Ok(());
            };
            let line = command.line();
            prop_assert!(line.starts_with("/rename "));
            prop_assert!(!line.chars().any(breaks_line), "{line:?}");
            prop_assert!(!line.chars().any(is_invisible_format), "{line:?}");
            prop_assert!(!line.ends_with('\\') && !line.ends_with(' '), "{line:?}");
            prop_assert!(
                line.chars().count() <= "/rename ".len() + MAX_ARGUMENT_CHARS
            );
            let [typed, enter] = command.keystrokes();
            prop_assert!(!typed.contains(&b'\r') && !typed.contains(&b'\n'));
            prop_assert_eq!(enter, SUBMIT.to_vec());
        }

        #[test]
        fn a_name_already_safe_is_typed_unchanged(raw in "[A-Za-z0-9][A-Za-z0-9 _.-]{0,40}[A-Za-z0-9]") {
            let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
            prop_assert_eq!(renamed(&raw), format!("/rename {collapsed}"));
        }
    }
}
