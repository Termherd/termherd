//! The `termherd://` links termherd answers, read from untrusted input.
//!
//! Any web page can hand termherd a link, so whatever this grammar does not
//! name is refused: a parse yields one request or an error, never a partial
//! reading.

/// What a link asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkRequest {
    /// Show the tab working in this repo, or open a shell there.
    OpenRepo(String),
}

/// Why a link was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("not a termherd:// link")]
    NotTermherd,
    #[error("unknown link action")]
    UnknownAction,
    #[error("the link names no repo")]
    MissingRepo,
    #[error("the link names more than one repo")]
    DuplicateRepo,
    #[error("unknown link parameter")]
    UnknownParameter,
    #[error("the repo path is not absolute")]
    RelativeRepo,
    #[error("malformed percent-encoding")]
    BadEncoding,
    #[error("the repo path holds a control character")]
    ControlCharacter,
    #[error("the repo path names a network share")]
    NetworkPath,
}

/// Read a `termherd://` link.
///
/// # Errors
/// Every [`LinkError`]: the link is refused whole.
pub fn parse(url: &str) -> Result<LinkRequest, LinkError> {
    let rest = strip_scheme(url).ok_or(LinkError::NotTermherd)?;
    let rest = rest
        .split_once('#')
        .map_or(rest, |(before, _fragment)| before);
    let (action, query) = rest.split_once('?').unwrap_or((rest, ""));
    if action.strip_suffix('/').unwrap_or(action) != "open" {
        return Err(LinkError::UnknownAction);
    }
    let encoded = repo_parameter(query)?;
    let repo = percent_decode(encoded)?;
    if repo.chars().any(char::is_control) {
        return Err(LinkError::ControlCharacter);
    }
    if repo.starts_with("//") || repo.starts_with("/\\") {
        // A UNC path on Windows: resolving it would reach out to that host.
        return Err(LinkError::NetworkPath);
    }
    let root = root_length(&repo).ok_or(LinkError::RelativeRepo)?;
    Ok(LinkRequest::OpenRepo(without_trailing_separator(
        repo, root,
    )))
}

const SCHEME: &str = "termherd://";

/// Whether `text` is addressed to termherd, valid or not.
#[must_use]
pub fn is_link(text: &str) -> bool {
    strip_scheme(text).is_some()
}

fn strip_scheme(url: &str) -> Option<&str> {
    let head = url.get(..SCHEME.len())?;
    head.eq_ignore_ascii_case(SCHEME)
        .then(|| &url[SCHEME.len()..])
}

/// The one `repo` value of `query`, still encoded.
fn repo_parameter(query: &str) -> Result<&str, LinkError> {
    let mut repo = None;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key != "repo" {
            return Err(LinkError::UnknownParameter);
        }
        if repo.replace(value).is_some() {
            return Err(LinkError::DuplicateRepo);
        }
    }
    repo.filter(|value| !value.is_empty())
        .ok_or(LinkError::MissingRepo)
}

/// RFC 3986 percent-decoding of the query (the fragment is dropped before): `+` stands for itself, and the bytes must
/// read as UTF-8.
fn percent_decode(encoded: &str) -> Result<String, LinkError> {
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut input = encoded.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = input.next().and_then(hex_value);
            let low = input.next().and_then(hex_value);
            let (Some(high), Some(low)) = (high, low) else {
                return Err(LinkError::BadEncoding);
            };
            bytes.push(high << 4 | low);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).map_err(|_| LinkError::BadEncoding)
}

fn hex_value(digit: u8) -> Option<u8> {
    char::from(digit)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}

/// The length of the root an absolute path starts with: `/` on Unix, a drive
/// and its separator on Windows. `None` for a relative path.
fn root_length(path: &str) -> Option<usize> {
    if path.starts_with('/') {
        return Some(1);
    }
    let mut chars = path.chars();
    let drive = chars.next()?.is_ascii_alphabetic();
    let colon = chars.next() == Some(':');
    let separator = matches!(chars.next(), Some('/' | '\\'));
    (drive && colon && separator).then_some(3)
}

/// Drop the separators a path ends with, keeping its root. A Unix path ends
/// only on `/`: a backslash there is part of a file name.
fn without_trailing_separator(mut path: String, root: usize) -> String {
    let separators: &[char] = if root == 1 { &['/'] } else { &['/', '\\'] };
    while path.len() > root && path.ends_with(separators) {
        path.pop();
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn open(repo: &str) -> Result<LinkRequest, LinkError> {
        Ok(LinkRequest::OpenRepo(repo.into()))
    }

    #[test]
    fn an_open_link_names_the_repo_to_show() {
        assert_eq!(
            parse("termherd://open?repo=/Users/me/repo"),
            open("/Users/me/repo")
        );
    }

    #[test]
    fn the_repo_is_percent_decoded() {
        assert_eq!(
            parse("termherd://open?repo=%2FUsers%2Fme%2Fmy%20repo"),
            open("/Users/me/my repo")
        );
        assert_eq!(
            parse("termherd://open?repo=/tmp/caf%C3%A9"),
            open("/tmp/café"),
            "a multi-byte sequence decodes as UTF-8"
        );
    }

    #[test]
    fn a_plus_is_a_plus_not_a_space() {
        // An HTML href is a URL, not a form body: `+` stands for itself.
        assert_eq!(parse("termherd://open?repo=/tmp/a+b"), open("/tmp/a+b"));
    }

    #[test]
    fn a_fragment_is_not_part_of_the_repo() {
        assert_eq!(parse("termherd://open?repo=/r#line-3"), open("/r"));
        assert_eq!(
            parse("termherd://open?repo=/r%23x"),
            open("/r#x"),
            "an encoded # is a #"
        );
    }

    #[test]
    fn the_scheme_is_case_insensitive() {
        assert_eq!(parse("TermHerd://open?repo=/r"), open("/r"));
    }

    #[test]
    fn a_trailing_slash_after_the_action_is_accepted() {
        // Some Windows browsers hand `termherd://open/?…` over for `termherd://open?…`.
        assert_eq!(parse("termherd://open/?repo=/r"), open("/r"));
    }

    #[test]
    fn a_trailing_slash_on_the_repo_is_dropped() {
        assert_eq!(parse("termherd://open?repo=/r/"), open("/r"));
        assert_eq!(
            parse("termherd://open?repo=/"),
            open("/"),
            "the root keeps its slash"
        );
    }

    #[test]
    fn a_windows_drive_path_is_absolute() {
        assert_eq!(
            parse("termherd://open?repo=C:%5CUsers%5Cme"),
            open("C:\\Users\\me")
        );
        assert_eq!(
            parse("termherd://open?repo=C:/Users/me"),
            open("C:/Users/me")
        );
    }

    #[test]
    fn anything_the_grammar_does_not_name_is_refused() {
        let cases = [
            ("https://open?repo=/r", LinkError::NotTermherd),
            ("termherd:open?repo=/r", LinkError::NotTermherd),
            ("termherd://session/abc", LinkError::UnknownAction),
            ("termherd://open", LinkError::MissingRepo),
            ("termherd://open?repo=", LinkError::MissingRepo),
            ("termherd://open?repo=/a&repo=/b", LinkError::DuplicateRepo),
            (
                "termherd://open?repo=/r&run=ls",
                LinkError::UnknownParameter,
            ),
            (
                "termherd://open?repo=relative/path",
                LinkError::RelativeRepo,
            ),
            ("termherd://open?repo=-flag", LinkError::RelativeRepo),
            ("termherd://open?repo=C:relative", LinkError::RelativeRepo),
            ("termherd://open?repo=/r%2", LinkError::BadEncoding),
            ("termherd://open?repo=/r%zz", LinkError::BadEncoding),
            ("termherd://open?repo=/r%FF", LinkError::BadEncoding),
            ("termherd://open?repo=/r%00", LinkError::ControlCharacter),
            ("termherd://open?repo=/r%0Als", LinkError::ControlCharacter),
            ("termherd://open?repo=//host/share", LinkError::NetworkPath),
            (
                "termherd://open?repo=%2F%2Fhost%2Fshare",
                LinkError::NetworkPath,
            ),
            (
                "termherd://open?repo=/%5Chost%5Cshare",
                LinkError::NetworkPath,
            ),
        ];
        for (url, error) in cases {
            assert_eq!(parse(url), Err(error), "{url}");
        }
    }

    #[test]
    fn a_link_is_told_by_its_scheme_alone() {
        assert!(is_link("termherd://open?repo=/r"));
        assert!(is_link("TERMHERD://anything"), "valid or not");
        assert!(!is_link("https://termherd.dev"));
        assert!(!is_link("termherd:"));
    }

    /// Every byte percent-encoded, so the round trip exercises the decoder on
    /// all of it.
    fn encode(path: &str) -> String {
        path.bytes().map(|byte| format!("%{byte:02X}")).collect()
    }

    proptest! {
        #[test]
        fn any_input_is_answered_without_a_panic(url in ".*") {
            let _ = parse(&url);
        }

        #[test]
        fn any_absolute_path_survives_the_round_trip(
            path in "/[^\\x00-\\x1F\\x7F-\\u{9F}/\\\\]{1,12}(/[^\\x00-\\x1F\\x7F-\\u{9F}/\\\\]{1,12}){0,4}",
        ) {
            let url = format!("termherd://open?repo={}", encode(&path));
            prop_assert_eq!(parse(&url), open(&path));
        }
    }
}
