//! Claude Code's input prompt, read off a terminal screen.
//!
//! How Claude Code draws its prompt — the `❯` marker, the rules framing it,
//! the placeholder hint — is a fact about its output format that moves with
//! its releases, so it lives in the codec beside the OSC signals. What to do
//! with the reading is `core`'s decision.

/// What Claude's input prompt holds, as read off the visible screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptInput {
    /// The prompt is on screen and empty, or shows only its placeholder hint.
    Empty,
    /// The prompt is on screen and holds this text, every line of it.
    Draft(String),
    /// No input prompt is on screen: a menu, a dialog or a picker has the
    /// keyboard, or the view is scrolled away from it. Enter would answer that
    /// instead of submitting a command.
    NotVisible,
}

/// Read Claude Code's input prompt off `screen` (the visible rows, one per
/// line).
///
/// The prompt is the row starting with `❯` (or `>` in older versions) that
/// sits right under a horizontal rule, and it runs to the next rule — a draft
/// of several lines continues on the rows between. Either rule may carry the
/// session's name as a label, `──── name ─`. An empty prompt shows a
/// placeholder hint, `Try "…"`, which counts as empty: the screen carries no
/// colour that would tell it from a draft spelling the same words, so a draft
/// of exactly that shape is the one this reads wrong.
#[must_use]
pub fn read_prompt(screen: &str) -> PromptInput {
    let rows: Vec<&str> = screen.lines().collect();
    let Some((start, first)) = rows
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, row)| prompt_text(row).map(|text| (index, text)))
    else {
        return PromptInput::NotVisible;
    };
    if start == 0 || !is_rule(rows[start - 1]) {
        return PromptInput::NotVisible;
    }
    let Some(len) = rows[start + 1..].iter().position(|row| is_rule(row)) else {
        return PromptInput::NotVisible;
    };
    let draft = std::iter::once(first)
        .chain(
            rows[start + 1..start + 1 + len]
                .iter()
                .map(|row| row.trim()),
        )
        .collect::<Vec<_>>()
        .join("\n");
    let draft = draft.trim();
    if draft.is_empty() || is_placeholder(draft) {
        PromptInput::Empty
    } else {
        PromptInput::Draft(draft.to_owned())
    }
}

/// The text after the prompt marker, when `row` is the prompt's first row.
///
/// Claude Code separates the marker from the text with a no-break space
/// (U+00A0), not an ASCII one, so any whitespace counts as the separator.
fn prompt_text(row: &str) -> Option<&str> {
    let rest = row.strip_prefix('❯').or_else(|| row.strip_prefix('>'))?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim())
}

/// Whether `row` is one of the horizontal rules that frame the prompt.
///
/// Once a session is named, Claude Code writes the name into a rule
/// (`──── name ─`), so a rule is a run of `─` at both ends with anything
/// between. The opening run must be long enough that a lone dash, or a line of
/// text that happens to start with one, is never read as a rule.
fn is_rule(row: &str) -> bool {
    const OPENING_RUN: usize = 3;
    let row = row.trim();
    row.chars().take(OPENING_RUN).filter(|&c| c == '─').count() == OPENING_RUN && row.ends_with('─')
}

/// Whether `text` is the hint an empty prompt shows, `Try "…"`.
fn is_placeholder(text: &str) -> bool {
    !text.contains('\n') && text.starts_with("Try \"") && text.ends_with('"') && text.len() > 5
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULE: &str = "────────────────────";

    fn screen(rows: &[&str]) -> String {
        rows.join("\n")
    }

    #[test]
    fn an_empty_prompt_showing_its_placeholder_reads_as_empty() {
        // As Claude Code draws it: the hint sits where a draft would.
        let text = screen(&[
            "  ◐ medium · /effort",
            RULE,
            "❯ Try \"fix lint errors\"",
            RULE,
            "  scratchpad │ Opus │ Ctx: 0",
        ]);
        assert_eq!(read_prompt(&text), PromptInput::Empty);
        assert_eq!(read_prompt(&screen(&[RULE, "❯", RULE])), PromptInput::Empty);
        assert_eq!(
            read_prompt(&screen(&[RULE, "> ", RULE])),
            PromptInput::Empty
        );
    }

    #[test]
    fn a_draft_of_several_lines_is_read_whole() {
        let text = screen(&[RULE, "❯ fix the\\", "  login bug", RULE, "  status"]);
        assert_eq!(
            read_prompt(&text),
            PromptInput::Draft("fix the\\\nlogin bug".to_owned())
        );
        assert_eq!(
            read_prompt(&screen(&[RULE, "❯ api", RULE])),
            PromptInput::Draft("api".to_owned())
        );
    }

    #[test]
    fn no_framed_prompt_on_screen_reads_as_not_visible() {
        // A picker, a dialog, or output scrolled over the prompt.
        for rows in [
            &["Select model", "❯ 1. Default", "  2. Opus"][..],
            &[RULE, "❯ unterminated"][..],
            &["❯ no rule above", RULE][..],
            &[][..],
            &[RULE, "❯x", RULE][..],
        ] {
            assert_eq!(
                read_prompt(&screen(rows)),
                PromptInput::NotVisible,
                "{rows:?}"
            );
        }
    }

    #[test]
    fn a_no_break_space_after_the_marker_still_reads_as_the_prompt() {
        // Claude Code draws `❯` then U+00A0, not an ASCII space: the row
        // below is the one captured from a live pane, byte for byte.
        let text = screen(&[
            RULE,
            "❯\u{a0}la saisie était visible, regarde la dernière capture",
            RULE,
            "  scratchpad │ Opus │ Ctx: 0",
        ]);
        assert_eq!(
            read_prompt(&text),
            PromptInput::Draft("la saisie était visible, regarde la dernière capture".to_owned())
        );
        assert_eq!(
            read_prompt(&screen(&[RULE, "❯\u{a0}", RULE])),
            PromptInput::Empty
        );
        assert_eq!(
            read_prompt(&screen(&[RULE, "❯\u{a0}Try \"fix lint errors\"", RULE])),
            PromptInput::Empty
        );
        assert_eq!(
            read_prompt(&screen(&[RULE, "❯\u{a0}fix it\u{a0}", RULE])),
            PromptInput::Draft("fix it".to_owned())
        );
    }

    #[test]
    fn a_rule_carrying_the_session_name_still_frames_the_prompt() {
        // After `/rename`, Claude Code writes the session name into the rule
        // above the prompt, right-aligned.
        let labelled = "───────────────────────────────────────── termherd un nom ─";
        let text = screen(&[
            labelled,
            "❯\u{a0}",
            RULE,
            "  termherd │ ⎇ main │ Opus 5.5 │ …",
        ]);
        assert_eq!(read_prompt(&text), PromptInput::Empty);
        assert_eq!(
            read_prompt(&screen(&[RULE, "❯\u{a0}fix it", labelled])),
            PromptInput::Draft("fix it".to_owned())
        );
        assert_eq!(
            read_prompt(&screen(&["───\u{a0}name\u{a0}─", "❯", RULE])),
            PromptInput::Empty
        );
    }

    #[test]
    fn a_row_that_only_looks_like_a_rule_does_not_frame_the_prompt() {
        for above in [
            "─",
            "── name ─",
            "plain text",
            "❯ 1. Default",
            "─── name",
            "name ───",
        ] {
            assert_eq!(
                read_prompt(&screen(&[above, "❯ draft", RULE])),
                PromptInput::NotVisible,
                "{above:?}"
            );
        }
    }

    #[test]
    fn the_lowest_prompt_on_screen_is_the_live_one() {
        // An earlier exchange can leave a framed `❯` line in the scrollback
        // above; the input box is always the last.
        let text = screen(&[RULE, "❯ old prompt", RULE, "answer", RULE, "❯", RULE]);
        assert_eq!(read_prompt(&text), PromptInput::Empty);
    }
}
