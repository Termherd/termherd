//! Shell-integration marks: the OSC 133 "semantic prompt" sequences a shell
//! emits around its prompt and each command it runs, and which are the only
//! thing that tells termherd a *plain shell* is working rather than parked.
//!
//! Claude's dialect (`termherd_claude::osc`) says nothing about a bare shell —
//! it decodes the CLI's own glyph titles, which a shell never writes. Without a
//! second dialect a `Launch::Shell` could never leave `Starting`, which is what
//! made `wait_for_status` unusable on one.
//!
//! The grammar is the FinalTerm / iTerm2 one every integrated terminal speaks:
//!
//! - `OSC 133 ; A` — a fresh prompt is being drawn; nothing is running.
//! - `OSC 133 ; B` — the prompt ended, user input starts. Carries no activity
//!   of its own (the shell is still parked), so it is not reported.
//! - `OSC 133 ; C` — the command was submitted and is now running.
//! - `OSC 133 ; D [ ; exit ]` — the command finished; back at the prompt.
//!
//! Stateless, like the Claude decoder: every mark found in a chunk is reported
//! in order, and a sequence split across two PTY chunks is not recognised.

use termherd_claude::osc::osc_sequences;

/// One shell-integration mark decoded from a PTY output chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptMark {
    /// The shell drew a prompt (`133;A`): it is parked, waiting for input.
    Ready,
    /// A command started (`133;C`): the shell is working.
    Running,
    /// A command finished (`133;D`): the shell is parked again.
    Done,
}

/// The OSC code shells report prompt and command boundaries under.
const SEMANTIC_PROMPT: u32 = 133;

/// Decode every shell-integration mark in one PTY output chunk.
pub(crate) fn decode_marks(chunk: &str) -> Vec<PromptMark> {
    // The scan is not free and the overwhelming majority of chunks are plain
    // output, so skip it unless something OSC-shaped is present at all.
    if !chunk.contains("\u{1b}]") {
        return Vec::new();
    }
    osc_sequences(chunk)
        .into_iter()
        .filter(|(code, _)| *code == SEMANTIC_PROMPT)
        .filter_map(|(_, payload)| mark_of(payload))
        .collect()
}

/// The mark an OSC 133 payload carries, or `None` for the kinds that report no
/// activity change — `B` (input starts, the shell is still parked) and any
/// letter a future shell adds.
fn mark_of(payload: &str) -> Option<PromptMark> {
    // `D` carries the command's exit status (`D;0`) and `A` may carry `aid=`
    // attributes, so only the leading letter classifies the mark.
    match payload.chars().next()? {
        'A' => Some(PromptMark::Ready),
        'C' => Some(PromptMark::Running),
        'D' => Some(PromptMark::Done),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_prompt_mark_says_the_shell_is_parked() {
        assert_eq!(
            decode_marks("\u{1b}]133;A\u{07}"),
            vec![PromptMark::Ready],
            "a drawn prompt is the shell reporting it is idle"
        );
    }

    #[test]
    fn a_command_start_mark_says_the_shell_is_working() {
        assert_eq!(
            decode_marks("\u{1b}]133;C\u{07}"),
            vec![PromptMark::Running]
        );
    }

    #[test]
    fn a_command_end_mark_carries_its_exit_status_and_still_decodes() {
        // Shells report `D;<exit>`; the status belongs to the command, not to
        // the session's activity, so it must not stop the mark from decoding.
        assert_eq!(decode_marks("\u{1b}]133;D;0\u{07}"), vec![PromptMark::Done]);
        assert_eq!(
            decode_marks("\u{1b}]133;D;130\u{07}"),
            vec![PromptMark::Done],
            "an interrupted command is still a finished one"
        );
    }

    #[test]
    fn the_input_boundary_mark_reports_no_activity() {
        // `B` only says "the prompt string ended"; the shell is still parked,
        // so reporting it would be a status change with nothing behind it.
        assert_eq!(decode_marks("\u{1b}]133;B\u{07}"), vec![]);
    }

    #[test]
    fn marks_are_reported_in_the_order_the_shell_wrote_them() {
        // One chunk routinely carries a whole command cycle.
        let chunk = "\u{1b}]133;C\u{07}some output\r\n\u{1b}]133;D;0\u{07}\u{1b}]133;A\u{07}";
        assert_eq!(
            decode_marks(chunk),
            vec![PromptMark::Running, PromptMark::Done, PromptMark::Ready]
        );
    }

    #[test]
    fn marks_survive_the_st_terminator_as_well_as_bel() {
        assert_eq!(
            decode_marks("\u{1b}]133;A\u{1b}\\"),
            vec![PromptMark::Ready],
            "ST-terminated sequences are as valid as BEL-terminated ones"
        );
    }

    #[test]
    fn an_unterminated_mark_is_not_decoded() {
        // A sequence cut by the chunk boundary is ambiguous; upstream's rule
        // is to ignore it rather than guess, so a half-mark changes nothing.
        assert_eq!(decode_marks("\u{1b}]133;A"), vec![]);
    }

    #[test]
    fn other_osc_codes_are_not_prompt_marks() {
        // A title and a hyperlink share the wire with the marks all day long.
        assert_eq!(decode_marks("\u{1b}]0;zsh in tmp\u{07}"), vec![]);
        assert_eq!(decode_marks("\u{1b}]8;;file:/tmp\u{07}"), vec![]);
        // And the near-miss codes must not be read as 133.
        assert_eq!(decode_marks("\u{1b}]13;A\u{07}"), vec![]);
        assert_eq!(decode_marks("\u{1b}]1337;A\u{07}"), vec![]);
    }

    proptest! {
        #[test]
        fn decoding_never_panics_on_arbitrary_output(chunk in ".*") {
            let _ = decode_marks(&chunk);
        }

        /// Whatever surrounds a mark, it is still found exactly once — the
        /// property that matters when a mark rides inside a noisy chunk.
        #[test]
        fn a_mark_is_found_whatever_plain_text_surrounds_it(
            before in "[^\u{1b}\u{07}]{0,32}",
            after in "[^\u{1b}\u{07}]{0,32}",
        ) {
            let chunk = format!("{before}\u{1b}]133;C\u{07}{after}");
            prop_assert_eq!(decode_marks(&chunk), vec![PromptMark::Running]);
        }
    }
}
