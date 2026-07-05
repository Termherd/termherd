//! OSC / control-sequence decoding — where session status comes from
//! (`F-status-notifications`, FR8) without any MCP bridge.
//!
//! Ported from the inline `ptyProcess.onData` parsing in the upstream
//! Electron app's `main.js` (`doctly/switchboard`). Claude CLI announces its
//! state through the terminal title and iTerm2-style sequences:
//!
//! - **OSC 0** (title): a Braille spinner char (U+2800–U+28FF) as the first
//!   title char means *busy*; `✳` (U+2733) means *idle, waiting for input*.
//! - **OSC 9;4** (progress): levels 1/2/3 mean *busy*. Level 0 also fires
//!   on clears, so upstream deliberately ignores it as an idle signal.
//! - **OSC 9** (other): a notification ("needs your attention", permission
//!   prompts, …) — the CLI only emits these when `TERM_PROGRAM` looks like
//!   iTerm2, which the pty adapter must arrange.
//! - **CSI ?1049h/l, ?47h/l**: alternate-screen enter/leave.
//!
//! This decoder is stateless: it reports every signal found in a chunk, in
//! upstream's processing order (OSC 0 first, then OSC 9, bell, alt-screen).
//! Upstream folds these into busy/idle *transitions* per session; that
//! state machine belongs in `core::App`, fed by these signals. Like
//! upstream, a sequence split across two PTY chunks is not recognised.

/// One status-relevant signal decoded from a PTY output chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OscSignal {
    /// Claude is working (spinner title or OSC 9;4 progress running).
    Busy,
    /// Claude is idle and waiting for input (`✳` title).
    Idle,
    /// The session title Claude reports in its OSC 0 sequence, with the
    /// leading busy/idle status glyph stripped. Empty titles are not reported.
    /// Only glyph-prefixed titles qualify — that filter is load-bearing: the
    /// shell emits junk titles first (`C:\…\cmd.exe …`, then `claude`) before
    /// Claude's real `✳ <name>`, and those must not reach the tab label.
    Title(String),
    /// OSC 9 notification payload (attention / permission requests).
    Notification(String),
    /// Alternate screen entered (`true`) or left (`false`).
    AltScreen(bool),
    /// A bare BEL outside any OSC sequence.
    Bell,
}

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

/// Decode every status signal in one PTY output chunk.
#[must_use]
pub fn decode_chunk(chunk: &str) -> Vec<OscSignal> {
    let mut signals = Vec::new();

    if chunk.contains("\u{1b}]") {
        let sequences = osc_sequences(chunk);
        // Pass 1 — OSC 0 titles (busy spinner / idle marker), in order. Each
        // status glyph also carries the human title text after it.
        for (code, payload) in &sequences {
            if *code != 0 {
                continue;
            }
            let status = match payload.chars().next() {
                Some(c) if ('\u{2800}'..='\u{28FF}').contains(&c) => Some(OscSignal::Busy),
                Some('\u{2733}') => Some(OscSignal::Idle),
                _ => None,
            };
            if let Some(status) = status {
                signals.push(status);
                if let Some(title) = title_after_glyph(payload) {
                    signals.push(OscSignal::Title(title.to_owned()));
                }
            }
        }
        // Pass 2 — OSC 9 progress and notifications, in order.
        for (code, payload) in &sequences {
            if *code != 9 {
                continue;
            }
            if let Some(progress) = payload.strip_prefix("4;") {
                let level = progress.split(';').next().unwrap_or("");
                // Level 0 doubles as "clear" → unreliable, ignored upstream.
                if matches!(level, "1" | "2" | "3") {
                    signals.push(OscSignal::Busy);
                }
            } else {
                signals.push(OscSignal::Notification((*payload).to_owned()));
            }
        }
    } else if chunk.contains('\u{07}') {
        signals.push(OscSignal::Bell);
    }

    if chunk.contains("\u{1b}[?") {
        if chunk.contains("\u{1b}[?1049h") || chunk.contains("\u{1b}[?47h") {
            signals.push(OscSignal::AltScreen(true));
        }
        if chunk.contains("\u{1b}[?1049l") || chunk.contains("\u{1b}[?47l") {
            signals.push(OscSignal::AltScreen(false));
        }
    }

    signals
}

/// The human title carried by an OSC 0 payload whose first char is a status
/// glyph (Braille spinner or `✳`): the text after that glyph, trimmed.
/// `None` when nothing meaningful remains — Claude's bare-glyph titles.
fn title_after_glyph(payload: &str) -> Option<&str> {
    let mut chars = payload.chars();
    chars.next()?; // drop the leading status glyph
    let rest = chars.as_str().trim();
    (!rest.is_empty()).then_some(rest)
}

/// All `ESC ] <digits> ; <payload>` sequences terminated by BEL or ST
/// (`ESC \`), mirroring `/\x1b\](\d+);([^\x07\x1b]*)(?:\x07|\x1b\\)/g`.
/// Sequences whose numeric code overflows `u32` are ignored.
fn osc_sequences(chunk: &str) -> Vec<(u32, &str)> {
    let bytes = chunk.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != ESC || bytes.get(i + 1) != Some(&b']') {
            i += 1;
            continue;
        }
        let digits_start = i + 2;
        let mut j = digits_start;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j == digits_start || bytes.get(j) != Some(&b';') {
            i += 1;
            continue;
        }
        let payload_start = j + 1;
        let mut k = payload_start;
        while k < bytes.len() && bytes[k] != BEL && bytes[k] != ESC {
            k += 1;
        }
        let end = if bytes.get(k) == Some(&BEL) {
            Some(k + 1)
        } else if bytes.get(k) == Some(&ESC) && bytes.get(k + 1) == Some(&b'\\') {
            Some(k + 2)
        } else {
            None // unterminated → not a match; rescan from the next byte
        };
        let Some(end) = end else {
            i += 1;
            continue;
        };
        // Slice boundaries sit on ASCII bytes, so they are char boundaries.
        if let Ok(code) = chunk[digits_start..j].parse::<u32>() {
            out.push((code, &chunk[payload_start..k]));
        }
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn braille_spinner_title_means_busy_and_carries_its_text() {
        // ⠋ U+280B — a spinner frame; the text after it is the live title.
        let chunk = "\u{1b}]0;\u{280B} Thinking…\u{07}";
        assert_eq!(
            decode_chunk(chunk),
            vec![OscSignal::Busy, OscSignal::Title("Thinking…".into())]
        );
    }

    #[test]
    fn asterisk_title_means_idle_and_carries_its_text() {
        let chunk = "\u{1b}]0;\u{2733} claude\u{07}";
        assert_eq!(
            decode_chunk(chunk),
            vec![OscSignal::Idle, OscSignal::Title("claude".into())]
        );
    }

    #[test]
    fn a_bare_status_glyph_carries_no_title() {
        // The spinner alone (no text after it) is a status with no title.
        assert_eq!(
            decode_chunk("\u{1b}]0;\u{280B}\u{07}"),
            vec![OscSignal::Busy]
        );
        assert_eq!(
            decode_chunk("\u{1b}]0;\u{2733}  \u{07}"),
            vec![OscSignal::Idle]
        );
    }

    #[test]
    fn plain_titles_are_not_status_signals() {
        let chunk = "\u{1b}]0;zsh\u{07}";
        assert_eq!(decode_chunk(chunk), vec![]);
    }

    #[test]
    fn st_terminator_is_accepted() {
        let chunk = "\u{1b}]0;\u{2733} claude\u{1b}\\";
        assert_eq!(
            decode_chunk(chunk),
            vec![OscSignal::Idle, OscSignal::Title("claude".into())]
        );
    }

    #[test]
    fn osc9_progress_running_means_busy_and_clear_is_ignored() {
        assert_eq!(decode_chunk("\u{1b}]9;4;1;50\u{07}"), vec![OscSignal::Busy]);
        assert_eq!(decode_chunk("\u{1b}]9;4;3;\u{07}"), vec![OscSignal::Busy]);
        assert_eq!(decode_chunk("\u{1b}]9;4;0;\u{07}"), vec![]);
    }

    #[test]
    fn osc9_payload_is_a_notification() {
        let chunk = "\u{1b}]9;Claude needs your attention\u{07}";
        assert_eq!(
            decode_chunk(chunk),
            vec![OscSignal::Notification(
                "Claude needs your attention".into()
            )]
        );
    }

    #[test]
    fn osc9_notification_keeps_semicolons_in_its_payload() {
        // Permission prompts carry `;` inside the human text. Only a
        // leading `4;` is the progress marker — inner semicolons must not split
        // the notification body.
        let chunk = "\u{1b}]9;allow Bash(rm -rf); proceed?\u{07}";
        assert_eq!(
            decode_chunk(chunk),
            vec![OscSignal::Notification(
                "allow Bash(rm -rf); proceed?".into()
            )]
        );
    }

    #[test]
    fn osc9_with_an_empty_payload_is_still_a_notification() {
        // A bare OSC 9 (no text) is a real attention ping; the empty body
        // is the core's to default, not the decoder's to drop.
        assert_eq!(
            decode_chunk("\u{1b}]9;\u{07}"),
            vec![OscSignal::Notification(String::new())]
        );
    }

    #[test]
    fn bare_bel_is_a_bell_but_osc_bel_is_not() {
        assert_eq!(decode_chunk("ding\u{07}"), vec![OscSignal::Bell]);
        // The BEL terminating an OSC sequence does not double as a bell.
        assert_eq!(decode_chunk("\u{1b}]0;zsh\u{07}"), vec![]);
    }

    #[test]
    fn alt_screen_transitions_are_reported() {
        assert_eq!(
            decode_chunk("\u{1b}[?1049h"),
            vec![OscSignal::AltScreen(true)]
        );
        assert_eq!(
            decode_chunk("\u{1b}[?47l"),
            vec![OscSignal::AltScreen(false)]
        );
        // Both in one chunk: upstream sets ON first, then OFF.
        assert_eq!(
            decode_chunk("\u{1b}[?1049h…\u{1b}[?1049l"),
            vec![OscSignal::AltScreen(true), OscSignal::AltScreen(false)]
        );
    }

    #[test]
    fn unterminated_sequences_are_ignored() {
        assert_eq!(decode_chunk("\u{1b}]0;half a title"), vec![]);
        // ESC not followed by backslash is not a terminator…
        assert_eq!(decode_chunk("\u{1b}]0;x\u{1b}[2J"), vec![]);
        // …but a later complete sequence is still found.
        let chunk = "\u{1b}]0;torn\u{1b}]0;\u{2733} ok\u{07}";
        assert_eq!(
            decode_chunk(chunk),
            vec![OscSignal::Idle, OscSignal::Title("ok".into())]
        );
    }

    #[test]
    fn signals_keep_upstream_processing_order() {
        // OSC 9 before OSC 0 in the stream — but pass order reports
        // title-derived signals first, like upstream's two regex passes.
        let chunk = "\u{1b}]9;note\u{07}\u{1b}]0;\u{280B}\u{07}";
        assert_eq!(
            decode_chunk(chunk),
            vec![OscSignal::Busy, OscSignal::Notification("note".into())]
        );
    }

    #[test]
    fn multibyte_payloads_do_not_break_slicing() {
        let chunk = "\u{1b}]0;émoji 🚀 title\u{07}\u{1b}]9;héhé\u{07}";
        assert_eq!(
            decode_chunk(chunk),
            vec![OscSignal::Notification("héhé".into())]
        );
    }

    proptest! {
        #[test]
        fn decode_never_panics(input in any::<String>()) {
            let _ = decode_chunk(&input);
        }

        #[test]
        fn arbitrary_notifications_roundtrip(payload in "[a-zA-Z0-9 àéü🚀]{0,80}") {
            let chunk = format!("\u{1b}]9;{payload}\u{07}");
            let signals = decode_chunk(&chunk);
            if payload.starts_with("4;") {
                // would be parsed as progress — not generated by this regex
                prop_assert!(true);
            } else {
                prop_assert_eq!(signals, vec![OscSignal::Notification(payload)]);
            }
        }
    }
}
