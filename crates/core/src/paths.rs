//! Pure file-path detection over a line of terminal text — the sibling of
//! [`crate::links`], same contract.
//!
//! Given one rendered grid row as a string, [`detect`] returns the spans that
//! *could* be a file path, as **character-index ranges** (start inclusive, end
//! exclusive). The terminal grid stores one `char` per cell, so a character
//! index is also the column.
//!
//! What comes back is **syntax, not truth**. `and/or`, `http/2` and `a/b` all
//! look exactly like relative paths, and no regex tells them apart from
//! `src/main.rs`. Only the filesystem does, which is I/O and therefore an
//! adapter's job ([`crate::ports::PathResolver`]). This scan's whole
//! responsibility is to be cheap, pure, and narrow enough that the check that
//! follows runs on a handful of candidates rather than on every word.

use core::ops::Range;

/// One path-shaped run found in a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSpan {
    /// The whole clickable run, suffix included — what gets underlined, so the
    /// `:184` a user aimed at is part of what lights up.
    pub range: Range<usize>,
    /// The path itself: `range` minus any `:line[:col]` suffix. This is what
    /// gets resolved against the filesystem.
    pub target: Range<usize>,
    /// The 1-based line number from a `:line` suffix, if any.
    pub line: Option<u32>,
    /// The 1-based column from a `:line:col` suffix, if any.
    pub col: Option<u32>,
}

/// Find the path-shaped spans in one line of terminal text.
///
/// A run qualifies when it carries a path separator (`/` or `\`) or a file
/// extension — a bare word like `main` does not, or every word on screen would
/// earn a filesystem probe. URLs are skipped: [`crate::links`] owns those, and
/// a scheme's `//` would otherwise read as a separator.
#[must_use]
pub fn detect(line: &str) -> Vec<PathSpan> {
    let chars: Vec<char> = line.chars().collect();
    let masked = url_mask(line, chars.len());
    let mut spans = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if masked[i] || !is_path_char(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !masked[i] && is_path_char(chars[i]) {
            i += 1;
        }
        if let Some(span) = span_of(&chars, start..i) {
            spans.push(span);
        }
    }
    spans
}

/// Which characters of `line` belong to a URL, so the path scan can treat them
/// as boundaries. Without this, `https://ex.io/a/b` is one long path-shaped run
/// — its scheme is alphanumeric and `:` and `/` are both path characters — and
/// the same cells would light up as two different kinds of link.
fn url_mask(line: &str, len: usize) -> Vec<bool> {
    let mut masked = vec![false; len];
    for span in crate::links::detect(line) {
        for cell in masked.get_mut(span).into_iter().flatten() {
            *cell = true;
        }
    }
    masked
}

/// Whether `c` can sit inside a path run. Deliberately the vocabulary a
/// double-click already treats as one word, so `~/src/main.rs:42` selects and
/// detects as the same unit. Whitespace and prose delimiters (`,`, quotes,
/// brackets) are boundaries; `:` is in, because a `:line` suffix has to survive
/// the scan to be peeled off after it.
fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '\\' | '~' | ':' | '@' | '+')
}

/// Turn one raw run into a span, or [`None`] when it is not path-shaped.
fn span_of(chars: &[char], raw: Range<usize>) -> Option<PathSpan> {
    let end = trim_trailing(chars, raw.start, raw.end);
    let range = raw.start..end;
    if range.is_empty() {
        return None;
    }
    let (target_end, line, col) = peel_position(chars, &range);
    let target = range.start..target_end;
    if target.is_empty() || !is_path_shaped(&chars[target.clone()]) {
        return None;
    }
    Some(PathSpan {
        range,
        target,
        line,
        col,
    })
}

/// Drop trailing characters that are valid inside a path but, at the very end,
/// are prose: the full stop closing a sentence and the colon a compiler puts
/// after a location. Both are in the run only because they are legal mid-path.
fn trim_trailing(chars: &[char], start: usize, mut end: usize) -> usize {
    while end > start && matches!(chars[end - 1], '.' | ':') {
        end -= 1;
    }
    end
}

/// Split a trailing `:line[:col]` off the run, returning where the path itself
/// ends. Only digits count, and only from the end — which is what keeps the `C:`
/// of a Windows drive from reading as a position.
fn peel_position(chars: &[char], range: &Range<usize>) -> (usize, Option<u32>, Option<u32>) {
    let (without_last, last) = trailing_number(chars, range.start, range.end);
    let Some(last) = last else {
        return (range.end, None, None);
    };
    // `path:line:col` — the second number peels only if a first one is behind
    // it, so `C:\a\b:9` reads as a line, not a column.
    let (without_both, first) = trailing_number(chars, range.start, without_last);
    first.map_or((without_last, Some(last), None), |first| {
        (without_both, Some(first), Some(last))
    })
}

/// Peel a `:<digits>` off the end of `start..end`, returning where it began and
/// its value. A run of digits that cannot be a line number is not a position —
/// truncating it would silently point at the wrong line.
fn trailing_number(chars: &[char], start: usize, end: usize) -> (usize, Option<u32>) {
    let mut digits = end;
    while digits > start && chars[digits - 1].is_ascii_digit() {
        digits -= 1;
    }
    if digits == end || digits == start || chars[digits - 1] != ':' {
        return (end, None);
    }
    let value: String = chars[digits..end].iter().collect();
    value
        .parse::<u32>()
        .map_or((end, None), |n| (digits - 1, Some(n)))
}

/// Whether a run is worth a filesystem probe: it carries a path separator, or
/// it is a dotted filename. A bare word is not — every word under the pointer
/// would otherwise cost a `stat`, which is the cost this predicate exists to
/// avoid.
fn is_path_shaped(target: &[char]) -> bool {
    // Separators alone name no file: a lone `/` exists on every Unix host, so
    // without this every mountpoint column of `df` output would underline.
    if !target.iter().any(|c| !matches!(c, '/' | '\\')) {
        return false;
    }
    if target.contains(&'/') || target.contains(&'\\') {
        return true;
    }
    // A dotted tail like `Cargo.toml`: the dot must separate two non-empty
    // parts, so a leading `.foo` (a hidden file, no extension) does not qualify
    // on its own.
    target
        .iter()
        .position(|&c| c == '.')
        .is_some_and(|dot| dot > 0 && dot + 1 < target.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The `(underlined, target, line, col)` tuples for readable assertions.
    fn found(line: &str) -> Vec<(String, String, Option<u32>, Option<u32>)> {
        let chars: Vec<char> = line.chars().collect();
        detect(line)
            .into_iter()
            .map(|s| {
                (
                    chars[s.range].iter().collect(),
                    chars[s.target].iter().collect(),
                    s.line,
                    s.col,
                )
            })
            .collect()
    }

    /// Just the underlined substrings.
    fn runs(line: &str) -> Vec<String> {
        found(line).into_iter().map(|(run, ..)| run).collect()
    }

    #[test]
    fn a_line_suffix_is_underlined_but_not_part_of_the_target() {
        // The motivating case: `cargo test` printing a failure location. The
        // user aims at `:184`, so it lights up; only the path is resolved.
        assert_eq!(
            found("thread panicked at crates/pty/src/grid.rs:184:"),
            [(
                "crates/pty/src/grid.rs:184".to_owned(),
                "crates/pty/src/grid.rs".to_owned(),
                Some(184),
                None,
            )]
        );
    }

    #[test]
    fn a_line_and_column_suffix_are_both_read() {
        assert_eq!(
            found("src/main.rs:42:7"),
            [(
                "src/main.rs:42:7".to_owned(),
                "src/main.rs".to_owned(),
                Some(42),
                Some(7),
            )]
        );
    }

    #[test]
    fn the_four_anchored_prefixes_are_candidates() {
        for line in ["/abs/path.rs", "./rel.rs", "../up.rs", "~/home.rs"] {
            assert_eq!(runs(line), [line], "{line} should be a candidate");
        }
    }

    #[test]
    fn a_bare_filename_with_an_extension_is_a_candidate() {
        // No separator, but a dotted tail: `Cargo.toml` printed alone is
        // exactly what a build error looks like.
        assert_eq!(runs("edit Cargo.toml now"), ["Cargo.toml"]);
    }

    #[test]
    fn a_dot_must_separate_two_non_empty_parts_to_count_as_an_extension() {
        // Both bounds of the dotted-filename rule, reached through a peeled
        // position suffix — which is the only way a target can still end in a
        // dot once trailing prose punctuation has been trimmed.
        //
        // A leading dot is a hidden file, not an extension: `.rs` names a
        // directory entry, and accepting it would make every `.` in prose a
        // probe. A trailing dot names nothing at all.
        assert!(
            runs(".rs:12").is_empty(),
            "a leading dot is not an extension"
        );
        assert!(
            runs("foo.:12").is_empty(),
            "a trailing dot is not one either"
        );
        // One character either side is enough — the bounds are exclusive of
        // the empty part, not of a short one.
        assert_eq!(runs("a.b:12"), ["a.b:12"]);
    }

    #[test]
    fn separators_alone_name_no_file() {
        // A lone `/` exists on every Unix host, so it would resolve and become
        // a link — underlining the mountpoint column of every `df`.
        assert!(runs("/").is_empty());
        assert!(runs("Filesystem  Size  Mounted on\n").is_empty());
        assert!(runs(r"\\").is_empty());
        // A separator with anything attached is still a candidate.
        assert_eq!(runs("/etc"), ["/etc"]);
    }

    #[test]
    fn a_bare_word_with_neither_separator_nor_extension_is_not_a_candidate() {
        // The cheapness guarantee: without this, every word under the pointer
        // would cost a filesystem probe.
        assert!(runs("just some plain text").is_empty());
        assert!(runs("main").is_empty());
    }

    #[test]
    fn a_windows_path_is_a_candidate_and_its_drive_is_not_a_line_suffix() {
        // `C:` is a drive letter, not `:line` — a suffix is only peeled off
        // the end, and only when it is digits.
        assert_eq!(
            found(r"C:\src\main.rs"),
            [(
                r"C:\src\main.rs".to_owned(),
                r"C:\src\main.rs".to_owned(),
                None,
                None,
            )]
        );
    }

    #[test]
    fn a_windows_path_still_takes_a_line_suffix() {
        assert_eq!(
            found(r"C:\src\main.rs:9"),
            [(
                r"C:\src\main.rs:9".to_owned(),
                r"C:\src\main.rs".to_owned(),
                Some(9),
                None,
            )]
        );
    }

    #[test]
    fn trailing_prose_punctuation_is_trimmed() {
        assert_eq!(runs("see src/main.rs."), ["src/main.rs"]);
        assert_eq!(runs("(see src/main.rs)"), ["src/main.rs"]);
        assert_eq!(runs("src/main.rs, then"), ["src/main.rs"]);
        // A dangling separator is not part of the path either.
        assert_eq!(runs("src/main.rs:"), ["src/main.rs"]);
    }

    #[test]
    fn a_url_is_not_a_path_candidate() {
        // `links::detect` owns URLs. Without this, `https://ex.io/a/b` reads as
        // a relative path and both would underline the same cells.
        assert!(runs("see https://ex.io/a/b here").is_empty());
        assert!(runs("file:///etc/hosts").is_empty());
    }

    #[test]
    fn prose_that_merely_looks_like_a_path_is_still_returned() {
        // Syntax cannot tell these from `src/main.rs`; the filesystem check
        // is what kills them. This test pins the division of labour — if it
        // ever starts passing, the filter moved into the wrong layer.
        assert_eq!(runs("and/or"), ["and/or"]);
        assert_eq!(runs("http/2 is fine"), ["http/2"]);
    }

    #[test]
    fn several_paths_on_one_line() {
        assert_eq!(runs("moved src/a.rs to src/b.rs"), ["src/a.rs", "src/b.rs"]);
    }

    #[test]
    fn ranges_are_char_indices_not_byte_indices() {
        // Leading multi-byte chars desync byte and char offsets; the returned
        // range must address chars so it maps onto grid columns.
        let line = "é→ src/main.rs:7";
        let spans = detect(line);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].range, 3..16);
        assert_eq!(spans[0].target, 3..14);
        let chars: Vec<char> = line.chars().collect();
        let got: String = chars[spans[0].target.clone()].iter().collect();
        assert_eq!(got, "src/main.rs");
    }

    #[test]
    fn an_absurd_line_number_is_dropped_rather_than_wrapping() {
        // A `:` followed by digits that cannot be a line number is not a
        // suffix — the whole run stays the target rather than silently
        // truncating to a wrapped value.
        let huge = format!("src/main.rs:{}", u64::from(u32::MAX) + 1);
        assert_eq!(found(&huge), [(huge.clone(), huge, None, None)]);
    }

    proptest! {
        /// Every returned range must be a valid char slice of the line, and
        /// `target` must sit inside `range`. A panic here is a slicing bug that
        /// would take the whole UI down on one odd line of terminal output.
        #[test]
        fn spans_are_always_valid_char_slices(line in ".{0,120}") {
            let chars: Vec<char> = line.chars().collect();
            for span in detect(&line) {
                prop_assert!(span.range.end <= chars.len());
                prop_assert!(span.range.start < span.range.end);
                prop_assert!(span.target.start >= span.range.start);
                prop_assert!(span.target.end <= span.range.end);
                prop_assert!(span.target.start < span.target.end);
            }
        }

        /// Spans never overlap and come back in order, so a caller can find the
        /// one under a column by a single scan and be sure it is unambiguous.
        #[test]
        fn spans_are_disjoint_and_increasing(line in ".{0,120}") {
            let spans = detect(&line);
            for pair in spans.windows(2) {
                prop_assert!(pair[0].range.end <= pair[1].range.start);
            }
        }

        /// A column inside a span resolves to exactly that span — the property
        /// the hover lookup relies on.
        #[test]
        fn every_column_belongs_to_at_most_one_span(line in ".{0,120}") {
            let spans = detect(&line);
            for col in 0..line.chars().count() {
                let hits = spans.iter().filter(|s| s.range.contains(&col)).count();
                prop_assert!(hits <= 1);
            }
        }
    }
}
