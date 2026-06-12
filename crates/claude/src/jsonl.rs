//! Shared JSONL line handling — the one place that decides how malformed
//! transcript lines are treated.
//!
//! **Deliberate deviation from upstream** (`read-session-file.js` runs
//! `JSON.parse` unguarded, so one corrupt line makes the whole session
//! vanish): torn lines are routine while the CLI is appending to a live
//! session, so a line that fails to parse is skipped and the rest of the
//! file is still used (Q5: predictable failure, never panic). Every JSONL
//! consumer in this crate goes through [`entries`] so the policy cannot
//! drift between them.

/// Iterate the JSON entries of JSONL content, skipping empty lines and
/// lines that fail to parse.
pub fn entries(jsonl: &str) -> impl Iterator<Item = serde_json::Value> + '_ {
    jsonl.lines().filter_map(|line| {
        if line.is_empty() {
            return None;
        }
        serde_json::from_str(line).ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_empty_and_corrupt_lines() {
        let jsonl = "\n{\"a\":1}\n{torn\n{\"b\":2}\n";
        let parsed: Vec<_> = entries(jsonl).collect();
        assert_eq!(
            parsed,
            vec![serde_json::json!({"a": 1}), serde_json::json!({"b": 2})]
        );
    }

    #[test]
    fn handles_crlf() {
        let parsed: Vec<_> = entries("{\"a\":1}\r\n{\"b\":2}\r\n").collect();
        assert_eq!(parsed.len(), 2);
    }
}
