//! The working directory a shell announces as it draws each prompt: the OSC 7
//! `file://` sequence every integrated terminal reads to follow a `cd`.
//!
//! Without it a session's directory is the one it was *launched* in, frozen for
//! the life of the terminal — so `PaneSnapshot.cwd` misreports the moment the
//! user types `cd`, and all four readers of it — the MCP snapshot (and the ⌘⇧S
//! capture sharing it), the directory a split inherits, the "new shell / new
//! Claude here" shortcuts, and the tab card — inherit that lie.
//!
//! Like the prompt marks in [`crate::prompt`], this is the *shell's* dialect,
//! not Claude's: the CLI never writes OSC 7. Only the wire scan is shared
//! (`termherd_claude::osc::osc_sequences`), which is the sharing that scan's
//! own doc-comment describes.
//!
//! Stateless: the announcement carried by a chunk is decoded from that chunk
//! alone, and a sequence split across two PTY chunks is not recognised.
//!
//! What the terminal writes is taken at its word: anything that reaches the
//! PTY — a `cat` of a crafted file, a build's output — can announce a
//! directory, so the reported path is where the session *says* it is and never
//! proof that a path exists. That is a **stronger primitive than the title**
//! the same reasoning covers, and worth saying plainly rather than by analogy:
//! a spoofed title is cosmetic, while `cwd` is where the next process starts —
//! a split and the "new shell / new Claude here" shortcuts launch there, and a
//! directory chooses which `.envrc`, git config and repo-local tooling apply.
//! Every integrated terminal carries the same exposure, and the same bound
//! keeps it honest: termherd only ever *starts* something in that directory in
//! response to the user asking for it, and never reads or writes a file there
//! off the announcement alone.

use termherd_claude::osc::osc_sequences;

/// The OSC code a shell reports its working directory under.
const WORKING_DIRECTORY: u32 = 7;

/// The working directory announced in one PTY output chunk, or `None` when the
/// chunk carries no usable announcement.
///
/// The last **usable** announcement wins — the newest one is the shell's
/// current answer, and a chunk routinely carries a whole `cd`-and-prompt cycle.
/// When the newest is not a local `file:` url the scan keeps walking backwards
/// rather than reporting nothing: an earlier announcement in the same chunk is
/// stale by milliseconds, where reporting nothing would leave the session on a
/// directory it left a `cd` ago.
pub(crate) fn decode_cwd(chunk: &str) -> Option<String> {
    // The scan is not free and the overwhelming majority of chunks are plain
    // output, so skip it unless something OSC-shaped is present at all.
    if !chunk.contains("\u{1b}]") {
        return None;
    }
    osc_sequences(chunk)
        .into_iter()
        .rev()
        .filter(|(code, _)| *code == WORKING_DIRECTORY)
        .find_map(|(_, payload)| path_of(payload))
}

/// The filesystem path carried by an OSC 7 payload — `file://<host><path>`.
///
/// The host is dropped: it names the machine the shell runs on, which the path
/// is only meaningful on. Anything but a `file:` url, and any url with no path
/// at all, announces nothing this machine can act on.
fn path_of(payload: &str) -> Option<String> {
    let authority = payload.strip_prefix("file://")?;
    let path = &authority[authority.find('/')?..];
    Some(tidy(&percent_decoded(path)))
}

/// One directory, spelled one way — so two spellings of the same place do not
/// read as a `cd` that never happened.
///
/// A trailing slash goes, since shells disagree on it — except where the slash
/// is all that makes the path a root: `/` on Unix, and `C:/` on Windows, where
/// a bare `C:` names the drive's *current* directory instead and would send the
/// next spawned shell somewhere else entirely. A Windows url also carries a
/// leading slash before the drive (`file:///C:/…`) that belongs to the url
/// rather than to the path.
fn tidy(path: &str) -> String {
    let path = path
        .strip_prefix('/')
        .filter(|rest| is_drive_rooted(rest))
        .unwrap_or(path);
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        // Nothing but separators: the root, however many were written.
        "/".to_owned()
    } else if is_drive_rooted(trimmed) && trimmed.len() == 2 {
        format!("{trimmed}/")
    } else {
        trimmed.to_owned()
    }
}

/// Whether `path` starts with a Windows drive letter (`C:/…`).
fn is_drive_rooted(path: &str) -> bool {
    let mut chars = path.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.next() == Some(':')
}

/// The payload with its `%XX` escapes resolved. Escapes are **bytes**, so they
/// are decoded into a buffer and read back as UTF-8 once — decoding them one at
/// a time would split every non-ASCII character into mojibake.
///
/// A `%` that starts no valid escape is kept as itself: it is a legal character
/// in a filename, and dropping the whole announcement over one would strand the
/// session on a directory the user has left.
fn percent_decoded(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match decoded_at(bytes, i) {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The byte the escape at `i` encodes, or `None` when `i` starts no escape.
fn decoded_at(bytes: &[u8], i: usize) -> Option<u8> {
    if bytes[i] != b'%' {
        return None;
    }
    let hex = bytes.get(i + 1..i + 3)?;
    let digit = |b: &u8| (*b as char).to_digit(16);
    Some((digit(&hex[0])? * 16 + digit(&hex[1])?) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// An OSC 7 announcement of `path` from `host`, as a shell writes it.
    fn announce(host: &str, path: &str) -> String {
        format!("\u{1b}]7;file://{host}{path}\u{07}")
    }

    #[test]
    fn a_shell_announcement_carries_the_directory_it_is_in() {
        assert_eq!(
            decode_cwd(&announce("laptop.local", "/Users/someone/code")),
            Some("/Users/someone/code".to_owned())
        );
    }

    #[test]
    fn the_host_is_optional_and_never_part_of_the_path() {
        // `file:///path` — the empty-host spelling, as common as the named one.
        assert_eq!(
            decode_cwd(&announce("", "/tmp/project")),
            Some("/tmp/project".to_owned())
        );
        // A named host must not leak into the path it prefixes.
        assert_eq!(
            decode_cwd(&announce("some.host", "/tmp/project")),
            Some("/tmp/project".to_owned())
        );
    }

    #[test]
    fn a_percent_escape_is_decoded_back_to_the_real_path() {
        // Shells percent-encode what a URL cannot carry literally; a directory
        // with a space is the everyday case.
        assert_eq!(
            decode_cwd(&announce("h", "/Users/some%20one/My%20Code")),
            Some("/Users/some one/My Code".to_owned())
        );
    }

    #[test]
    fn a_multibyte_escape_is_decoded_as_one_character_not_two_bytes() {
        // Percent escapes are *bytes*: `é` is two of them, and decoding them
        // one at a time would produce mojibake rather than the path.
        assert_eq!(
            decode_cwd(&announce("h", "/tmp/caf%C3%A9")),
            Some("/tmp/café".to_owned())
        );
        assert_eq!(
            decode_cwd(&announce("h", "/tmp/%F0%9F%9A%80")),
            Some("/tmp/🚀".to_owned())
        );
    }

    #[test]
    fn a_truncated_escape_is_kept_literally_rather_than_dropping_the_path() {
        // A stray `%` is legal in a filename and illegal in a URL. Whichever it
        // is, losing the whole announcement over it would strand the session on
        // a stale directory — far worse than one odd character.
        assert_eq!(
            decode_cwd(&announce("h", "/tmp/100%")),
            Some("/tmp/100%".to_owned())
        );
        assert_eq!(
            decode_cwd(&announce("h", "/tmp/a%2")),
            Some("/tmp/a%2".to_owned())
        );
        assert_eq!(
            decode_cwd(&announce("h", "/tmp/%zz")),
            Some("/tmp/%zz".to_owned())
        );
    }

    #[test]
    fn both_terminators_are_accepted() {
        assert_eq!(
            decode_cwd("\u{1b}]7;file://h/tmp\u{1b}\\"),
            Some("/tmp".to_owned()),
            "ST-terminated sequences are as valid as BEL-terminated ones"
        );
    }

    #[test]
    fn a_trailing_slash_does_not_make_a_different_directory() {
        // zsh's `%d` and bash's `$PWD` disagree on the trailing slash at the
        // root; two spellings of one directory would look like a `cd` that
        // never happened.
        assert_eq!(decode_cwd(&announce("h", "/tmp/x/")), Some("/tmp/x".into()));
        assert_eq!(
            decode_cwd(&announce("h", "/")),
            Some("/".to_owned()),
            "the root is a directory, not an empty path"
        );
        assert_eq!(
            decode_cwd(&announce("", "//")),
            Some("/".to_owned()),
            "a path that is nothing but separators is still the root"
        );
    }

    #[test]
    fn a_windows_drive_path_loses_the_url_leading_slash() {
        // `file:///C:/Users/x` is the standard spelling; the slash belongs to
        // the URL, not to the path, and would make it unopenable.
        assert_eq!(
            decode_cwd(&announce("", "/C:/Users/someone")),
            Some("C:/Users/someone".to_owned())
        );
    }

    #[test]
    fn a_windows_drive_root_keeps_the_slash_that_makes_it_a_root() {
        // `C:` alone is not the root of the drive: it names that drive's
        // *current* directory, so trimming the slash off `C:/` would send the
        // next shell launched here somewhere else entirely.
        assert_eq!(
            decode_cwd(&announce("", "/C:/")),
            Some("C:/".to_owned()),
            "a drive root is its slash, exactly as `/` is on Unix"
        );
        assert_eq!(
            decode_cwd(&announce("", "/C:/Users/")),
            Some("C:/Users".to_owned()),
            "below the root the trailing slash is still noise"
        );
    }

    #[test]
    fn the_last_announcement_in_a_chunk_wins() {
        // One chunk routinely carries a `cd` and the prompt that follows it.
        let chunk = format!("{}output\r\n{}", announce("h", "/a"), announce("h", "/b"));
        assert_eq!(decode_cwd(&chunk), Some("/b".to_owned()));
    }

    #[test]
    fn other_osc_codes_are_not_directory_announcements() {
        // Titles, prompt marks and hyperlinks share the wire with OSC 7.
        assert_eq!(decode_cwd("\u{1b}]0;zsh in tmp\u{07}"), None);
        assert_eq!(decode_cwd("\u{1b}]133;A\u{07}"), None);
        assert_eq!(decode_cwd("\u{1b}]8;;file:///tmp/x\u{07}"), None);
        // And the near-miss codes must not be read as 7.
        assert_eq!(decode_cwd("\u{1b}]77;file:///tmp\u{07}"), None);
        assert_eq!(decode_cwd("\u{1b}]17;file:///tmp\u{07}"), None);
    }

    #[test]
    fn an_announcement_that_is_not_a_local_file_url_is_ignored() {
        // Anything but a `file:` URL names a directory that is not on this
        // machine — following it would point the session at a path that does
        // not exist here.
        assert_eq!(decode_cwd("\u{1b}]7;http://example.com/tmp\u{07}"), None);
        assert_eq!(decode_cwd("\u{1b}]7;/tmp/no-scheme\u{07}"), None);
        assert_eq!(decode_cwd("\u{1b}]7;\u{07}"), None);
        assert_eq!(
            decode_cwd("\u{1b}]7;file://justahost\u{07}"),
            None,
            "a host with no path announces no directory"
        );
    }

    #[test]
    fn an_unterminated_announcement_is_not_decoded() {
        // A sequence cut by the chunk boundary is ambiguous; the decoders here
        // ignore it rather than guess at half a path.
        assert_eq!(decode_cwd("\u{1b}]7;file:///tmp/hal"), None);
        // …but a later complete one is still found.
        let chunk = format!("\u{1b}]7;file:///tmp/torn{}", announce("h", "/done"));
        assert_eq!(decode_cwd(&chunk), Some("/done".to_owned()));
    }

    proptest! {
        #[test]
        fn decoding_never_panics_on_arbitrary_output(chunk in ".*") {
            let _ = decode_cwd(&chunk);
        }

        /// Whatever a shell percent-encodes, the path comes back — the property
        /// that matters, since the encoder is the user's shell and not ours.
        #[test]
        fn an_encoded_path_survives_the_round_trip(
            segment in "[-a-zA-Z0-9_ éü🚀%?#]{1,24}",
        ) {
            let path = format!("/tmp/{segment}");
            let encoded: String = path
                .bytes()
                .map(|b| match b {
                    b'/' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                    b if b.is_ascii_alphanumeric() => (b as char).to_string(),
                    b => format!("%{b:02X}"),
                })
                .collect();
            prop_assert_eq!(
                decode_cwd(&announce("host", &encoded)),
                Some(path.trim_end_matches('/').to_owned())
            );
        }

        /// A mark rides inside noisy output all day long.
        #[test]
        fn an_announcement_is_found_whatever_surrounds_it(
            before in "[^\u{1b}\u{07}]{0,32}",
            after in "[^\u{1b}\u{07}]{0,32}",
        ) {
            let chunk = format!("{before}{}{after}", announce("h", "/tmp/x"));
            prop_assert_eq!(decode_cwd(&chunk), Some("/tmp/x".to_owned()));
        }
    }
}
