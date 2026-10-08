//! The shape of a Claude session id, checked wherever one is about to become
//! an argument on the `claude` command line termherd types into a shell.

/// Whether `id` is a well-formed Claude session id: the charset Claude Code
/// mints (`[A-Za-z0-9_-]`), non-empty, and not starting with `-`. A session id
/// becomes the `--resume <id>` termherd types into the shell, so anything
/// outside this charset is refused rather than trusted in a shell grammar that
/// differs per platform — and a leading `-` (e.g. `--help`, `-rf`) is refused
/// too, since `claude` would parse it as a flag rather than the value.
#[must_use]
pub fn is_valid(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Whether `id` has the shape `claude --session-id` takes: a hyphenated UUID
/// (8-4-4-4-12 hex digits). Every such id is also [`is_valid`].
#[must_use]
pub fn is_uuid(id: &str) -> bool {
    const HYPHENS: [usize; 4] = [8, 13, 18, 23];
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if HYPHENS.contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_session_id_is_only_the_claude_charset() {
        for ok in [
            "abc",
            "9f8e7d6c-4b2a-4c1d-8e3f-0123456789ab",
            "with_underscore",
            "MiXeD-123",
        ] {
            assert!(is_valid(ok), "{ok:?} is a well-formed id");
        }
        for bad in [
            "",       // empty
            "a b",    // space
            "x;rm",   // command separator
            "x$(id)", // command substitution
            "a`id`",  // backtick
            "a|b",    // pipe
            "a/b",    // path separator
            "a'b",    // quote
            "a\nb",   // newline
            "-rf",    // leading dash → `claude --resume -rf` reads it as a flag
            "--help", // ditto, a real claude flag
            "-",      // bare dash
        ] {
            assert!(!is_valid(bad), "{bad:?} must be refused");
        }
    }

    #[test]
    fn a_uuid_is_exactly_the_hyphenated_hex_shape() {
        assert!(is_uuid("0b9f2c4e-7d1a-4e8b-9c3f-5a6d7e8f9012"));
        for bad in [
            "",
            "--help",
            "-0b9f2c4e-7d1a-4e8b-9c3f-5a6d7e8f901",
            "abc-123",
            "0b9f2c4e-7d1a-4e8b-9c3f-5a6d7e8f901z",
            "0b9f2c4e_7d1a_4e8b_9c3f_5a6d7e8f9012",
            "0b9f2c4e-7d1a-4e8b-9c3f-5a6d7e8f9012; rm -rf ~",
        ] {
            assert!(!is_uuid(bad), "{bad:?} must be refused");
        }
    }

    proptest! {
        #[test]
        fn every_uuid_is_a_valid_id_and_nothing_flag_shaped_is_either(
            id in "[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
            flag in "-.*",
        ) {
            prop_assert!(is_uuid(&id) && is_valid(&id));
            prop_assert!(!is_uuid(&flag) && !is_valid(&flag));
        }
    }
}
