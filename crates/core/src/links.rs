//! Pure URL detection over a line of terminal text.
//!
//! Given one rendered grid row as a string, [`detect`] returns the spans that
//! are clickable URLs, as **character-index ranges** (start inclusive, end
//! exclusive). The terminal grid stores one `char` per cell, so a character
//! index is also the column — the shell maps a returned range straight onto
//! cell columns for hover-highlighting and click resolution.
//!
//! No I/O, no allocation beyond the result: detection is a pure scan so it is
//! exhaustively unit-testable (Q5), matching the `core` quality bar.

use core::ops::Range;

/// URL schemes we recognise as clickable. Order matters only in that no scheme
/// is a prefix of another here, so the first match at a position is correct.
const SCHEMES: [&str; 4] = ["https://", "http://", "file://", "ftp://"];

/// Find the clickable URL spans in one line of terminal text.
///
/// Ranges are character indices into `line` (== grid columns). Each span begins
/// at a scheme (`https://`, `http://`, `file://`, `ftp://`) sitting on a word
/// boundary and runs to the first character that cannot belong to a URL.
/// Trailing sentence punctuation and unbalanced closing brackets are trimmed so
/// prose like `(see https://example.com).` yields just the bare URL.
#[must_use]
pub fn detect(line: &str) -> Vec<Range<usize>> {
    let chars: Vec<char> = line.chars().collect();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match scheme_at(&chars, i) {
            // A scheme must not sit mid-word (e.g. `nothttps://x`), so the
            // preceding character must not be alphanumeric.
            Some(scheme_len) if i == 0 || !chars[i - 1].is_alphanumeric() => {
                let body_start = i + scheme_len;
                let mut end = body_start;
                while end < chars.len() && is_url_char(chars[end]) {
                    end += 1;
                }
                let raw_end = end;
                let end = trim_trailing(&chars, i, body_start, end);
                // A scheme with no host (`https://` alone, or only trimmable
                // junk after it) is not a link.
                if end > body_start {
                    spans.push(i..end);
                }
                // Resume past the whole raw run so an inner `http` can't
                // re-trigger inside a URL we already consumed.
                i = raw_end.max(i + 1);
            }
            _ => i += 1,
        }
    }
    spans
}

/// The length of the URL scheme starting at `chars[i]`, if any.
fn scheme_at(chars: &[char], i: usize) -> Option<usize> {
    SCHEMES.iter().find_map(|scheme| {
        let len = scheme.chars().count();
        let matches = chars[i..]
            .iter()
            .zip(scheme.chars())
            .filter(|(a, b)| a.eq_ignore_ascii_case(b))
            .count()
            == len;
        matches.then_some(len)
    })
}

/// Whether `c` can appear in the body of a URL (the unreserved + reserved set,
/// minus whitespace, controls, and prose delimiters like `"`, `<`, `>`).
fn is_url_char(c: char) -> bool {
    if c.is_whitespace() || c.is_control() {
        return false;
    }
    matches!(c,
        'a'..='z' | 'A'..='Z' | '0'..='9'
        | '-' | '.' | '_' | '~' | ':' | '/' | '?' | '#'
        | '[' | ']' | '@' | '!' | '$' | '&' | '\'' | '(' | ')'
        | '*' | '+' | ',' | ';' | '=' | '%')
}

/// Trim trailing characters that are valid URL bytes but, at the very end, are
/// almost always prose: sentence punctuation and unbalanced closing brackets.
/// `start` is the scheme start (for bracket balance), `body_start` the floor we
/// never trim below.
fn trim_trailing(chars: &[char], start: usize, body_start: usize, mut end: usize) -> usize {
    while end > body_start {
        let c = chars[end - 1];
        let drop = match c {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' => true,
            ')' => unbalanced(chars, start, end, '(', ')'),
            ']' => unbalanced(chars, start, end, '[', ']'),
            _ => false,
        };
        if drop {
            end -= 1;
        } else {
            break;
        }
    }
    end
}

/// Whether the closing bracket at `end - 1` has no matching opener within
/// `start..end` — i.e. it belongs to the surrounding prose, not the URL.
fn unbalanced(chars: &[char], start: usize, end: usize, open: char, close: char) -> bool {
    let opens = chars[start..end].iter().filter(|&&c| c == open).count();
    let closes = chars[start..end].iter().filter(|&&c| c == close).count();
    closes > opens
}

#[cfg(test)]
mod tests {
    use super::detect;

    /// The substrings `detect` marks as links, for readable assertions.
    fn links(line: &str) -> Vec<String> {
        let chars: Vec<char> = line.chars().collect();
        detect(line)
            .into_iter()
            .map(|r| chars[r].iter().collect())
            .collect()
    }

    #[test]
    fn plain_https_url() {
        assert_eq!(
            links("see https://example.com here"),
            ["https://example.com"]
        );
    }

    #[test]
    fn url_with_path_query_and_fragment() {
        let line = "https://a.b/c/d?e=f&g=h#frag";
        assert_eq!(links(line), [line]);
    }

    #[test]
    fn trailing_sentence_punctuation_is_trimmed() {
        assert_eq!(links("Visit https://example.com."), ["https://example.com"]);
        assert_eq!(links("https://example.com, then"), ["https://example.com"]);
    }

    #[test]
    fn url_wrapped_in_parens_drops_the_closing_paren() {
        assert_eq!(links("(see https://example.com)"), ["https://example.com"]);
    }

    #[test]
    fn balanced_parens_inside_the_url_are_kept() {
        let line = "https://en.wikipedia.org/wiki/Foo_(bar)";
        assert_eq!(links(line), [line]);
    }

    #[test]
    fn http_and_file_and_ftp_schemes() {
        assert_eq!(links("http://x.y"), ["http://x.y"]);
        assert_eq!(links("file:///etc/hosts"), ["file:///etc/hosts"]);
        assert_eq!(links("ftp://host/path"), ["ftp://host/path"]);
    }

    #[test]
    fn scheme_is_matched_case_insensitively() {
        assert_eq!(
            links("HTTPS://Example.COM/Path"),
            ["HTTPS://Example.COM/Path"]
        );
    }

    #[test]
    fn multiple_urls_on_one_line() {
        assert_eq!(
            links("a http://one.com b https://two.com c"),
            ["http://one.com", "https://two.com"]
        );
    }

    #[test]
    fn no_scheme_means_no_links() {
        assert!(links("example.com is not clickable").is_empty());
        assert!(links("just some plain text").is_empty());
    }

    #[test]
    fn scheme_mid_word_is_not_a_link() {
        assert!(links("nothttps://example.com").is_empty());
    }

    #[test]
    fn bare_scheme_with_no_host_is_not_a_link() {
        assert!(links("https://").is_empty());
        assert!(links("look: https:// nothing").is_empty());
    }

    #[test]
    fn ranges_are_char_indices_not_byte_indices() {
        // Leading multi-byte chars would desync byte and char offsets; the
        // returned range must address chars so it maps onto grid columns.
        let line = "é→ https://x.io";
        let span = detect(line);
        assert_eq!(span, vec![3..15]);
        let chars: Vec<char> = line.chars().collect();
        let got: String = chars[span[0].clone()].iter().collect();
        assert_eq!(got, "https://x.io");
    }
}
