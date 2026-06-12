//! Project-path encoding mirrors Claude CLI's folder-naming scheme so we read
//! and write the same folder names the CLI does for any given project path.
//!
//! Reverse-engineered from claude CLI 2.1.126 (ported from the upstream
//! Electron app's `encode-project-path.js`, `doctly/switchboard`).

/// Encode a project path into the folder name Claude CLI uses under
/// `~/.claude/projects/`.
///
/// Rules (matched byte-for-byte against the JS reference):
///
/// 1. Replace each UTF-16 code unit that is not `[a-zA-Z0-9]` with `-`.
/// 2. If the sanitized form is ≤ 200 code units, return it.
/// 3. Otherwise, compute a djb2-style 32-bit hash of the *original* path
///    (`h = (h << 5) - h + unit` per UTF-16 code unit, wrapping `i32`) and
///    return `sanitized[..200] + "-" + |h| in base 36`.
pub fn encode_project_path(project_path: &str) -> String {
    // Sanitize over UTF-16 code units to match the JS reference exactly.
    let mut sanitized = String::new();
    for unit in project_path.encode_utf16() {
        let mapped = match char::from_u32(u32::from(unit)) {
            Some(c) if c.is_ascii_alphanumeric() => c,
            _ => '-',
        };
        sanitized.push(mapped);
    }

    if sanitized.chars().count() <= 200 {
        return sanitized;
    }

    // 32-bit wrapping hash, JS-compatible: (h << 5) - h + unit.
    let mut h: i32 = 0;
    for unit in project_path.encode_utf16() {
        h = h
            .wrapping_shl(5)
            .wrapping_sub(h)
            .wrapping_add(i32::from(unit));
    }
    // `Math.abs(i32::MIN)` in JS yields 2^31 (since the result is f64); to
    // match, widen to i64 before taking the magnitude.
    let abs = i64::from(h).unsigned_abs();

    let mut out: String = sanitized.chars().take(200).collect();
    out.push('-');
    out.push_str(&to_base36(abs));
    out
}

fn to_base36(mut n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut digits: Vec<u8> = Vec::with_capacity(13);
    while n > 0 {
        let d = (n % 36) as u8;
        let c = if d < 10 { b'0' + d } else { b'a' + (d - 10) };
        digits.push(c);
        n /= 36;
    }
    digits.reverse();
    String::from_utf8(digits).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_path_is_just_sanitized() {
        let out = encode_project_path("/Users/alice/dev/proj");
        assert_eq!(out, "-Users-alice-dev-proj");
    }

    #[test]
    fn special_chars_become_dashes() {
        // dots, slashes, spaces, tildes — all non-alphanumeric → '-'.
        let out = encode_project_path("/a.b/c d/e~f");
        assert_eq!(out, "-a-b-c-d-e-f");
    }

    #[test]
    fn alphanumerics_pass_through_unchanged() {
        let out = encode_project_path("AaZz09");
        assert_eq!(out, "AaZz09");
    }

    #[test]
    fn empty_input_yields_empty_output() {
        assert_eq!(encode_project_path(""), "");
    }

    #[test]
    fn long_path_gets_truncated_with_hash_suffix() {
        let long = "/".to_string() + &"a".repeat(300);
        let out = encode_project_path(&long);
        assert!(out.len() > 200, "expected hash suffix beyond 200 chars");
        let (prefix, suffix) = out.split_at(200);
        // Sanitized prefix: '-' for the leading '/', then 199 'a's.
        assert_eq!(&prefix[..1], "-");
        assert_eq!(&prefix[1..], "a".repeat(199).as_str());
        // Suffix begins with '-' and is ASCII-base36 after that.
        assert!(suffix.starts_with('-'));
        assert!(
            suffix[1..]
                .chars()
                .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()),
            "hash suffix must be base36"
        );
    }

    #[test]
    fn hash_is_deterministic() {
        let p = "/some/very/long/path/".to_string() + &"x".repeat(300);
        assert_eq!(encode_project_path(&p), encode_project_path(&p));
    }
}
