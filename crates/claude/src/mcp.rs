//! The `mcpServers` config the Claude CLI reads — one more of its file formats,
//! rendered here so the two places that produce it cannot drift into two
//! dialects of the same thing.
//!
//! A session termherd spawns receives this through `--mcp-config`; a client
//! that attaches from outside gets the identical text from
//! `termherd mcp-config`. The second producer arrived long after the first, and
//! a hand-copied format string is exactly the kind of duplication that stays
//! correct until one side gains a field.

/// The single-server `mcpServers` config pointing at `url`, authenticated with
/// `token`, as one line of JSON.
///
/// Both fields are known-safe inside a bare JSON string — a loopback url and a
/// hex token — so neither is escaped. Anything else would need a real encoder.
#[must_use]
pub fn servers_config(url: &str, token: &str) -> String {
    format!(
        r#"{{"mcpServers":{{"termherd":{{"type":"http","url":"{url}","headers":{{"Authorization":"Bearer {token}"}}}}}}}}"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_names_the_url_and_bearer_token() {
        assert_eq!(
            servers_config("http://127.0.0.1:52431/mcp", "0123456789abcdef"),
            r#"{"mcpServers":{"termherd":{"type":"http","url":"http://127.0.0.1:52431/mcp","headers":{"Authorization":"Bearer 0123456789abcdef"}}}}"#
        );
    }

    /// The Claude CLI rejects a config it cannot parse, and the failure surfaces
    /// as a session that launches without its bridge rather than as an error
    /// here — so the shape is asserted as JSON, not only as bytes.
    #[test]
    fn the_config_parses_back_to_the_two_fields() {
        let text = servers_config("http://127.0.0.1:1/mcp", "abc");
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        let server = &value["mcpServers"]["termherd"];
        assert_eq!(server["type"], "http");
        assert_eq!(server["url"], "http://127.0.0.1:1/mcp");
        assert_eq!(server["headers"]["Authorization"], "Bearer abc");
    }
}
