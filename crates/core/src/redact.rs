//! Mask the secrets a terminal shows before its text is written to disk.
//!
//! Only shapes a credential announces itself by are masked: a known prefix, an
//! `AWS_…KEY=` assignment, a `Bearer` header, a PEM private key. No entropy
//! guess — it would also mask the hashes, UUIDs and base64 a terminal shows all
//! day, and a capture is read to see what the screen said.

use std::ops::Range;

/// What a masked secret is replaced with.
pub const MASK: &str = "[redacted]";

/// `text` with every recognised secret replaced by [`MASK`]. Everything else is
/// kept byte for byte, and the result is never longer than `text`.
#[must_use]
pub fn redact(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut at = 0;
    while at < bytes.len() {
        // Right after a mask, the byte before `at` is the mask's, not the
        // secret's: reading the original would let a second pass match what
        // the first one refused, and redaction would not be idempotent.
        let before = if at == copied {
            out.as_bytes().last().copied()
        } else {
            Some(bytes[at - 1])
        };
        match secret_at(bytes, at, before) {
            Some(span) => {
                out.push_str(&text[copied..span.start]);
                out.push_str(MASK);
                copied = span.end;
                at = span.end;
            }
            None => at += 1,
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// The span to mask if a secret starts at `at`. Every prefix is ASCII, so a
/// match never starts or ends inside a multi-byte character.
fn secret_at(bytes: &[u8], at: usize, before: Option<u8>) -> Option<Range<usize>> {
    if before.is_some_and(is_word) {
        return None;
    }
    let token =
        |prefix: &[u8], min: usize, body: fn(u8) -> bool| token_at(bytes, at, prefix, min, body);
    token(b"AKIA", 16, is_upper_alnum)
        .or_else(|| token(b"ASIA", 16, is_upper_alnum))
        .or_else(|| token(b"sk-", 20, is_key))
        .or_else(|| GITHUB_PREFIXES.iter().find_map(|p| token(p, 20, is_alnum)))
        .or_else(|| token(b"github_pat_", 20, is_word))
        .or_else(|| bearer_at(bytes, at))
        .or_else(|| aws_assignment_at(bytes, at))
        .or_else(|| private_key_at(bytes, at))
}

/// GitHub's token prefixes: personal, OAuth, user-to-server, server-to-server,
/// refresh.
const GITHUB_PREFIXES: [&[u8]; 5] = [b"ghp_", b"gho_", b"ghu_", b"ghs_", b"ghr_"];

/// The shortest value masked after a `Bearer` or an `AWS_…KEY=`: real ones are
/// far longer, and a floor above [`MASK`]'s length keeps a capture from ever
/// growing.
const MIN_VALUE: usize = 16;

/// `prefix` followed by at least `min` bytes of `body`: the whole run is the
/// secret.
fn token_at(
    bytes: &[u8],
    at: usize,
    prefix: &[u8],
    min: usize,
    body: fn(u8) -> bool,
) -> Option<Range<usize>> {
    let start = at + prefix.len();
    if !bytes[at..].starts_with(prefix) {
        return None;
    }
    let end = run_end(bytes, start, body);
    (end - start >= min).then_some(at..end)
}

/// `Bearer <token>`, in any case: the token is masked, the scheme kept.
fn bearer_at(bytes: &[u8], at: usize) -> Option<Range<usize>> {
    const SCHEME: &[u8] = b"bearer ";
    let scheme = bytes.get(at..at + SCHEME.len())?;
    if !scheme.eq_ignore_ascii_case(SCHEME) {
        return None;
    }
    let start = at + SCHEME.len();
    let end = run_end(bytes, start, is_bearer_token);
    (end - start >= MIN_VALUE).then_some(start..end)
}

/// `AWS_<NAME>=<value>` where the name says it holds a key, a secret or a
/// token: the value is masked, quotes included, the name kept. `AWS_REGION`
/// and its like say where, not who, and stay readable.
fn aws_assignment_at(bytes: &[u8], at: usize) -> Option<Range<usize>> {
    if !bytes[at..].starts_with(b"AWS_") {
        return None;
    }
    let name_end = run_end(bytes, at, |b| {
        b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'
    });
    let name = &bytes[at..name_end];
    let names_a_secret = [&b"KEY"[..], b"SECRET", b"TOKEN"]
        .iter()
        .any(|word| name.windows(word.len()).any(|w| w == *word));
    if !names_a_secret || bytes.get(name_end) != Some(&b'=') {
        return None;
    }
    let start = name_end + 1;
    let (end, inner) = match bytes.get(start) {
        Some(&quote @ (b'"' | b'\'')) => {
            let close = bytes[start + 1..].iter().position(|&b| b == quote);
            close.map_or_else(
                || {
                    let end = run_end(bytes, start, |b| !b.is_ascii_whitespace());
                    (end, end - start)
                },
                |close| (start + close + 2, close),
            )
        }
        _ => {
            let end = run_end(bytes, start, |b| !b.is_ascii_whitespace());
            (end, end - start)
        }
    };
    (inner >= MIN_VALUE).then_some(start..end)
}

/// A PEM private key: the base64 between its `BEGIN` and `END` lines is masked
/// and both lines kept, so the reader still sees what was on screen. A key the
/// screen cut off before its `END` line is masked to the end of the text.
fn private_key_at(bytes: &[u8], at: usize) -> Option<Range<usize>> {
    const BEGIN: &[u8] = b"-----BEGIN ";
    const DASHES: &[u8] = b"-----";
    if !bytes[at..].starts_with(BEGIN) {
        return None;
    }
    let label_start = at + BEGIN.len();
    let line_end = find(bytes, label_start, b"\n").unwrap_or(bytes.len());
    let label_end = find(&bytes[..line_end], label_start, DASHES)?;
    if !bytes[label_start..label_end].ends_with(b"PRIVATE KEY") {
        return None;
    }
    let body_start = label_end + DASHES.len();
    let body_end = find(bytes, body_start, b"-----END ").unwrap_or(bytes.len());
    let body = &bytes[body_start..body_end];
    let first = body.iter().position(|b| !b.is_ascii_whitespace())?;
    let last = body.iter().rposition(|b| !b.is_ascii_whitespace())?;
    let span = body_start + first..body_start + last + 1;
    (span.len() >= MASK.len()).then_some(span)
}

/// The first index at or after `from` where `needle` starts.
fn find(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|offset| from + offset)
}

/// Where the run of `body` bytes starting at `from` ends.
fn run_end(bytes: &[u8], from: usize, body: impl Fn(u8) -> bool) -> usize {
    bytes
        .get(from..)
        .and_then(|rest| rest.iter().position(|&b| !body(b)))
        .map_or(bytes.len(), |offset| from + offset)
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_alnum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

fn is_upper_alnum(b: u8) -> bool {
    b.is_ascii_uppercase() || b.is_ascii_digit()
}

fn is_key(b: u8) -> bool {
    is_word(b) || b == b'-'
}

/// RFC 6750's `b64token` alphabet.
fn is_bearer_token(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// A fake AWS access key id: the `AKIA` prefix and 16 upper-case
    /// alphanumerics.
    const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
    const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

    #[test]
    fn each_secret_shape_is_masked_and_its_surroundings_kept() {
        let cases = [
            (
                "aws access key id",
                format!("id {ACCESS_KEY} ok"),
                "id [redacted] ok".to_owned(),
            ),
            (
                "aws temporary access key id",
                "ASIAY34FZKBOKMUTVV7A".to_owned(),
                "[redacted]".to_owned(),
            ),
            (
                "aws secret assignment",
                format!("AWS_SECRET_ACCESS_KEY={SECRET_KEY}\nnext"),
                "AWS_SECRET_ACCESS_KEY=[redacted]\nnext".to_owned(),
            ),
            (
                "quoted aws session token",
                "export AWS_SESSION_TOKEN=\"IQoJb3JpZ2luX2VjEJr//////////wEaCXVz\" done".to_owned(),
                "export AWS_SESSION_TOKEN=[redacted] done".to_owned(),
            ),
            (
                "openai style key",
                "key: sk-proj-abcdefghijklmnopqrstuvwx1234".to_owned(),
                "key: [redacted]".to_owned(),
            ),
            (
                "anthropic key",
                "ANTHROPIC_API_KEY=sk-ant-api03-AbCdEfGhIjKlMnOpQrStUv_wx-yz".to_owned(),
                "ANTHROPIC_API_KEY=[redacted]".to_owned(),
            ),
            (
                "github classic token",
                "ghp_1234567890abcdefghijABCDEFGHIJ123456 pushed".to_owned(),
                "[redacted] pushed".to_owned(),
            ),
            (
                "github oauth token",
                "gho_16C7e42F292c6912E7710c838347Ae178B4a".to_owned(),
                "[redacted]".to_owned(),
            ),
            (
                "github fine-grained token",
                "(github_pat_11ABCDEFG0123456789_abcdefghijklmnop)".to_owned(),
                "([redacted])".to_owned(),
            ),
            (
                "bearer header",
                "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig".to_owned(),
                "Authorization: Bearer [redacted]".to_owned(),
            ),
            (
                "lower-case bearer",
                "-H 'authorization: bearer 0123456789abcdef0123'".to_owned(),
                "-H 'authorization: bearer [redacted]'".to_owned(),
            ),
            (
                "pem private key",
                "$ cat id\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\nQyNTUxOQAAACD\n-----END OPENSSH PRIVATE KEY-----\n$ ".to_owned(),
                "$ cat id\n-----BEGIN OPENSSH PRIVATE KEY-----\n[redacted]\n-----END OPENSSH PRIVATE KEY-----\n$ ".to_owned(),
            ),
            (
                "pem private key cut off by the screen",
                "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA1234567890\nabcdef".to_owned(),
                "-----BEGIN RSA PRIVATE KEY-----\n[redacted]".to_owned(),
            ),
            (
                "two secrets on one line",
                format!("{ACCESS_KEY} ghp_1234567890abcdefghijABCDEFGHIJ123456"),
                "[redacted] [redacted]".to_owned(),
            ),
        ];
        for (name, input, expected) in cases {
            assert_eq!(redact(&input), expected, "{name}");
        }
    }

    #[test]
    fn look_alikes_are_left_alone() {
        let cases = [
            ("a word ending in sk-", "task-runner"),
            ("a short sk- word", "sk-learn installed"),
            (
                "an access key inside a longer word",
                "XAKIAIOSFODNN7EXAMPLE",
            ),
            ("too short for an access key", "AKIA1234"),
            (
                "an aws setting that is not a secret",
                "AWS_REGION=eu-west-3",
            ),
            (
                "an aws secret name without a value",
                "unset AWS_SECRET_ACCESS_KEY",
            ),
            ("a short aws value", "AWS_SECRET_ACCESS_KEY=changeme"),
            ("bearer in prose", "the Bearer of this note"),
            (
                "a public key",
                "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE\n-----END PUBLIC KEY-----",
            ),
            (
                "a git hash",
                "commit 4001cb0f1e2d3c4b5a69788796a5b4c3d2e1f0a9",
            ),
            ("a uuid", "session 3f2b8c1e-9d4a-4b7e-8c2f-1a0b9e8d7c6f"),
            ("a short github prefix", "ghp_tooshort"),
            ("non-ascii text", "café ☕ — ünïcödé sk-"),
        ];
        for (name, input) in cases {
            assert_eq!(redact(input), input, "{name}");
        }
    }

    /// A secret of each recognised shape, with its prefix varied.
    fn secret() -> impl Strategy<Value = String> {
        prop_oneof![
            "AKIA[A-Z0-9]{16}",
            "sk-[A-Za-z0-9_-]{20,40}",
            "gh[pousr]_[A-Za-z0-9]{20,40}",
            "github_pat_[A-Za-z0-9_]{20,40}",
            "Bearer [A-Za-z0-9._~+/=-]{16,40}",
            "AWS_SECRET_ACCESS_KEY=[A-Za-z0-9/+]{16,40}",
            "-----BEGIN EC PRIVATE KEY-----\n[A-Za-z0-9+/=]{16,64}(\n[A-Za-z0-9+/=]{1,64}){0,3}\n-----END EC PRIVATE KEY-----",
        ]
    }

    /// Text that holds no secret: lower-case words and punctuation, without
    /// the `b` that would let it spell `bearer`, so no rule's prefix can be
    /// spelled from it.
    fn plain() -> impl Strategy<Value = String> {
        "[ac-z0-9 .,:;()\n\u{e9}\u{2615}]{0,40}"
    }

    proptest! {
        #[test]
        fn text_without_a_secret_is_unchanged(text in plain()) {
            prop_assert_eq!(redact(&text), text);
        }

        #[test]
        fn a_secret_is_masked_and_what_surrounds_it_is_kept(
            before in plain(),
            secret in secret(),
            after in plain(),
        ) {
            let text = format!("{before} {secret} {after}");
            let masked = redact(&text);
            let before_prefix = format!("{before} ");
            let after_suffix = format!(" {after}");
            prop_assert!(masked.starts_with(&before_prefix), "{masked:?}");
            prop_assert!(masked.ends_with(&after_suffix), "{masked:?}");
            prop_assert!(masked.contains(MASK), "{masked:?}");
            prop_assert!(masked.len() <= text.len(), "{masked:?} is longer");
        }

        #[test]
        fn redaction_is_idempotent_and_never_lengthens(
            text in prop::collection::vec(prop_oneof![plain(), secret()], 0..6)
                .prop_map(|parts| parts.join(" ")),
        ) {
            let once = redact(&text);
            prop_assert!(once.len() <= text.len(), "{once:?} is longer than {text:?}");
            prop_assert_eq!(redact(&once), once.clone());
        }

        #[test]
        fn arbitrary_text_never_grows(text in any::<String>()) {
            let once = redact(&text);
            prop_assert!(once.len() <= text.len());
            prop_assert_eq!(redact(&once), once.clone());
        }
    }
}
