//! Session digest — the lightweight JSONL parse behind the session browser
//! and full-text search (`F-session-browser`, `F-fts-search`).
//!
//! Ported from `read-session-file.js` (upstream Electron app,
//! `doctly/switchboard`). The caller supplies the file *content*; session id
//! (file stem) and created/modified timestamps come from the filesystem and
//! are the scan adapter's job.
//!
//! Malformed lines are skipped, not fatal — the policy (a deliberate
//! deviation from upstream) lives in [`crate::jsonl`].

/// What the browser and the FTS index need from one session JSONL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionDigest {
    /// First real user prompt, truncated to 120 UTF-16 units.
    pub summary: String,
    /// Number of user + assistant messages.
    pub message_count: u32,
    /// Concatenated message texts for full-text indexing (~8 KB cap).
    pub text_content: String,
    /// Session slug, from the first line that carries one.
    pub slug: Option<String>,
    /// Manual title (`custom-title` entry); the last one wins.
    pub custom_title: Option<String>,
    /// AI-generated title (`ai-title` entry); the last one wins.
    pub ai_title: Option<String>,
    /// The last few message snippets (oldest→newest), one condensed line each,
    /// for a "where did this session end" preview in the sidebar tooltip. Kept
    /// separate from [`Self::text_content`] so it captures the true *tail* even
    /// when the indexed text hit its size cap mid-session.
    pub tail: Vec<String>,
    /// The Claude Code version (`version`) of the last user, assistant or
    /// system entry that records one.
    pub version: Option<String>,
    /// The model (`message.model`) of the last assistant reply that names one.
    /// Sidechain and synthetic replies are skipped: neither is the model the
    /// session talks to.
    pub model: Option<String>,
    /// The reasoning effort of the last assistant reply that records one:
    /// `effort`, else `perTurnEffort`. Neither is documented, and older
    /// Claude Code versions write neither.
    pub effort: Option<String>,
}

impl SessionDigest {
    /// Title precedence — the contract, pinned here so no caller can
    /// re-derive it differently: user rename > custom title > AI title >
    /// summary. Empty strings count as absent (JS `||` semantics).
    #[must_use]
    pub fn display_title<'a>(&'a self, user_rename: Option<&'a str>) -> &'a str {
        user_rename
            .and_then(non_empty)
            .or_else(|| self.custom_title.as_deref().and_then(non_empty))
            .or_else(|| self.ai_title.as_deref().and_then(non_empty))
            .unwrap_or(&self.summary)
    }
}

fn non_empty(s: &str) -> Option<&str> {
    if s.is_empty() { None } else { Some(s) }
}

/// A string field of a transcript entry, with JS-truthiness semantics:
/// missing, non-string, or empty all count as absent.
fn non_empty_str<'a>(entry: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    entry
        .get(key)
        .and_then(serde_json::Value::as_str)
        .and_then(non_empty)
}

/// Maximum UTF-16 length of the summary (JS `text.slice(0, 120)`).
const SUMMARY_MAX_UNITS: usize = 120;
/// Per-message contribution to `text_content` (JS `text.slice(0, 500)`).
const TEXT_SLICE_UNITS: usize = 500;
/// `text_content` stops growing once it reaches this many UTF-16 units.
/// Upstream checks *before* appending, so the final size may exceed it.
const TEXT_CONTENT_MAX_UNITS: usize = 8000;
/// How many trailing message snippets [`SessionDigest::tail`] keeps.
const TAIL_MESSAGES: usize = 3;
/// Max UTF-16 length of one [`SessionDigest::tail`] snippet.
const TAIL_SNIPPET_UNITS: usize = 100;

/// Digest one session JSONL. Returns `None` when the session is not worth
/// listing — no real user prompt, or no messages at all (mirrors upstream's
/// `if (!summary || messageCount < 1) return null`).
#[must_use]
pub fn digest_session(content: &str) -> Option<SessionDigest> {
    let mut summary = String::new();
    let mut message_count: u32 = 0;
    let mut text_content = String::new();
    let mut text_content_units: usize = 0;
    let mut slug: Option<String> = None;
    let mut custom_title: Option<String> = None;
    let mut ai_title: Option<String> = None;
    let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut run = RunFacts::default();

    for entry in crate::jsonl::entries(content) {
        if slug.is_none()
            && let Some(s) = non_empty_str(&entry, "slug")
        {
            slug = Some(s.to_owned());
        }

        let entry_type = entry.get("type").and_then(serde_json::Value::as_str);
        let role = entry.get("role").and_then(serde_json::Value::as_str);

        if entry_type == Some("custom-title")
            && let Some(t) = non_empty_str(&entry, "customTitle")
        {
            custom_title = Some(t.to_owned());
        }
        if entry_type == Some("ai-title")
            && let Some(t) = non_empty_str(&entry, "aiTitle")
        {
            ai_title = Some(t.to_owned());
        }

        let is_user =
            entry_type == Some("user") || (entry_type == Some("message") && role == Some("user"));
        let is_assistant = entry_type == Some("assistant")
            || (entry_type == Some("message") && role == Some("assistant"));
        if is_user || is_assistant {
            message_count = message_count.saturating_add(1);
        }
        run.observe(
            &entry,
            is_user || is_assistant || entry_type == Some("system"),
            is_assistant,
        );

        let text = message_text(&entry);

        if summary.is_empty() && is_user && !text.is_empty() && !is_local_command(text) {
            // Scheduled runs get a stable name instead of the prompt text.
            summary = match scheduled_task_name(text) {
                Some(name) => format!("Scheduled: {name}"),
                None => utf16_prefix(text, SUMMARY_MAX_UNITS).to_owned(),
            };
        }

        if !text.is_empty() && text_content_units < TEXT_CONTENT_MAX_UNITS {
            let slice = utf16_prefix(text, TEXT_SLICE_UNITS);
            text_content.push_str(slice);
            text_content.push('\n');
            text_content_units += slice.encode_utf16().count() + 1;
        }

        // Rolling tail of the last few message lines, kept regardless of the
        // text_content cap so it reflects how the session actually ended.
        if (is_user || is_assistant)
            && let Some(line) = first_nonempty_line(text)
        {
            tail.push_back(utf16_prefix(line, TAIL_SNIPPET_UNITS).to_owned());
            if tail.len() > TAIL_MESSAGES {
                tail.pop_front();
            }
        }
    }

    if summary.is_empty() || message_count < 1 {
        return None;
    }
    Some(SessionDigest {
        summary,
        message_count,
        text_content,
        slug,
        custom_title,
        ai_title,
        tail: tail.into(),
        version: run.version,
        model: run.model,
        effort: run.effort,
    })
}

/// What a transcript says about the Claude that wrote it, each field the last
/// value an entry recorded.
#[derive(Default)]
struct RunFacts {
    version: Option<String>,
    model: Option<String>,
    effort: Option<String>,
}

/// The model Claude Code names on a reply it made up itself (an interrupted
/// or failed turn), which no API call answered.
const SYNTHETIC_MODEL: &str = "<synthetic>";

impl RunFacts {
    fn observe(&mut self, entry: &serde_json::Value, carries_version: bool, is_assistant: bool) {
        if carries_version && let Some(v) = non_empty_str(entry, "version") {
            self.version = Some(v.to_owned());
        }
        let sidechain = entry
            .get("isSidechain")
            .and_then(serde_json::Value::as_bool);
        if !is_assistant || sidechain == Some(true) {
            return;
        }
        let model = entry.get("message").and_then(|m| non_empty_str(m, "model"));
        if model == Some(SYNTHETIC_MODEL) {
            return;
        }
        if let Some(model) = model {
            self.model = Some(model.to_owned());
        }
        if let Some(effort) =
            non_empty_str(entry, "effort").or_else(|| non_empty_str(entry, "perTurnEffort"))
        {
            self.effort = Some(effort.to_owned());
        }
    }
}

/// First line of `text` with surrounding whitespace stripped, skipping leading
/// blank lines — the one-line condensation used for [`SessionDigest::tail`].
fn first_nonempty_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

/// The text of a transcript entry, mirroring the upstream lookup chain:
/// `message` itself if it is a string, else `message.content` if a string,
/// else `message.content[0].text`, else empty.
fn message_text(entry: &serde_json::Value) -> &str {
    let Some(msg) = entry.get("message") else {
        return "";
    };
    if let Some(s) = msg.as_str() {
        return s;
    }
    if let Some(content) = msg.get("content") {
        if let Some(s) = content.as_str() {
            return s;
        }
        if let Some(s) = content
            .get(0)
            .and_then(|first| first.get("text"))
            .and_then(serde_json::Value::as_str)
        {
            return s;
        }
    }
    ""
}

/// Local `!`-command artefacts that must not become the summary
/// (JS `/<bash-input>|<bash-stdout>|<local-command-caveat>/`).
fn is_local_command(text: &str) -> bool {
    text.contains("<bash-input>")
        || text.contains("<bash-stdout>")
        || text.contains("<local-command-caveat>")
}

/// The `name` attribute of a `<scheduled-task name="...">` tag, mirroring
/// `/<scheduled-task\s+name="([^"]+)"/` — at least one whitespace between
/// the tag and the attribute, and a non-empty quoted name.
fn scheduled_task_name(text: &str) -> Option<&str> {
    const TAG: &str = "<scheduled-task";
    let mut search = text;
    while let Some(pos) = search.find(TAG) {
        let rest = &search[pos + TAG.len()..];
        let after_ws = rest.trim_start();
        if after_ws.len() < rest.len()
            && let Some(quoted) = after_ws.strip_prefix("name=\"")
            && let Some(end) = quoted.find('"')
            && end > 0
        {
            return Some(&quoted[..end]);
        }
        search = rest;
    }
    None
}

/// Longest prefix of `s` that fits in `max_units` UTF-16 code units without
/// splitting a character. (JS `slice` can cut a surrogate pair in half;
/// Rust strings cannot represent that, so we stop one unit earlier.)
fn utf16_prefix(s: &str, max_units: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        let w = c.len_utf16();
        if units + w > max_units {
            return &s[..i];
        }
        units += w;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn user_line(text: &str) -> String {
        serde_json::json!({ "type": "user", "message": text }).to_string()
    }

    #[test]
    fn minimal_session_digests() {
        let jsonl = user_line("hello world");
        let d = digest_session(&jsonl).unwrap();
        assert_eq!(d.summary, "hello world");
        assert_eq!(d.message_count, 1);
        assert_eq!(d.text_content, "hello world\n");
        assert_eq!(d.slug, None);
    }

    #[test]
    fn tail_keeps_the_last_three_messages_as_condensed_lines() {
        let jsonl = [
            user_line("first"),
            serde_json::json!({"type":"assistant","message":"second"}).to_string(),
            user_line("third"),
            serde_json::json!({"type":"assistant","message":"  \nfourth line one\nignored"})
                .to_string(),
        ]
        .join("\n");
        let d = digest_session(&jsonl).unwrap();
        // Only the last three survive, each condensed to its first non-blank
        // line (leading blank lines and trailing lines dropped).
        assert_eq!(d.tail, vec!["second", "third", "fourth line one"]);
    }

    #[test]
    fn message_type_with_role_counts() {
        let jsonl = [
            serde_json::json!({"type":"message","role":"user","message":"hi"}).to_string(),
            serde_json::json!({"type":"message","role":"assistant","message":"yo"}).to_string(),
            serde_json::json!({"type":"message","role":"system","message":"x"}).to_string(),
        ]
        .join("\n");
        let d = digest_session(&jsonl).unwrap();
        assert_eq!(d.message_count, 2);
        assert_eq!(d.summary, "hi");
    }

    #[test]
    fn nested_content_text_is_found() {
        let jsonl = serde_json::json!({
            "type": "user",
            "message": { "content": [ { "type": "text", "text": "nested" } ] }
        })
        .to_string();
        let d = digest_session(&jsonl).unwrap();
        assert_eq!(d.summary, "nested");
    }

    #[test]
    fn string_content_is_found() {
        let jsonl = serde_json::json!({
            "type": "user",
            "message": { "content": "flat" }
        })
        .to_string();
        assert_eq!(digest_session(&jsonl).unwrap().summary, "flat");
    }

    #[test]
    fn local_commands_do_not_become_the_summary() {
        let jsonl = [
            user_line("<bash-input>ls</bash-input>"),
            user_line("real prompt"),
        ]
        .join("\n");
        let d = digest_session(&jsonl).unwrap();
        assert_eq!(d.summary, "real prompt");
        assert_eq!(d.message_count, 2);
    }

    #[test]
    fn scheduled_task_gets_a_stable_summary() {
        let jsonl = user_line(r#"<scheduled-task name="nightly build"> run it"#);
        let d = digest_session(&jsonl).unwrap();
        assert_eq!(d.summary, "Scheduled: nightly build");
    }

    #[test]
    fn scheduled_task_requires_whitespace_and_a_name() {
        assert_eq!(scheduled_task_name(r#"<scheduled-taskname="x">"#), None);
        assert_eq!(scheduled_task_name(r#"<scheduled-task name="">"#), None);
        assert_eq!(
            scheduled_task_name(r#"<scheduled-task  name="ok">"#),
            Some("ok")
        );
        // A bogus first tag does not stop the scan from finding a later one.
        assert_eq!(
            scheduled_task_name(r#"<scheduled-taskish> <scheduled-task name="ok">"#),
            Some("ok")
        );
    }

    #[test]
    fn titles_and_slug_are_captured_with_their_precedence_rules() {
        let jsonl = [
            serde_json::json!({"slug":"first-slug"}).to_string(),
            serde_json::json!({"slug":"second-slug"}).to_string(),
            user_line("prompt"),
            serde_json::json!({"type":"custom-title","customTitle":"old"}).to_string(),
            serde_json::json!({"type":"custom-title","customTitle":"new"}).to_string(),
            serde_json::json!({"type":"ai-title","aiTitle":"ai"}).to_string(),
        ]
        .join("\n");
        let d = digest_session(&jsonl).unwrap();
        // slug: first wins; titles: last wins.
        assert_eq!(d.slug.as_deref(), Some("first-slug"));
        assert_eq!(d.custom_title.as_deref(), Some("new"));
        assert_eq!(d.ai_title.as_deref(), Some("ai"));
    }

    #[test]
    fn display_title_precedence_is_the_46_contract() {
        let jsonl = [
            user_line("the prompt"),
            serde_json::json!({"type":"custom-title","customTitle":"custom"}).to_string(),
            serde_json::json!({"type":"ai-title","aiTitle":"ai"}).to_string(),
        ]
        .join("\n");
        let d = digest_session(&jsonl).unwrap();
        assert_eq!(d.display_title(Some("rename")), "rename");
        assert_eq!(d.display_title(Some("")), "custom");
        assert_eq!(d.display_title(None), "custom");
        let mut no_custom = d.clone();
        no_custom.custom_title = None;
        assert_eq!(no_custom.display_title(None), "ai");
        no_custom.ai_title = None;
        assert_eq!(no_custom.display_title(None), "the prompt");
    }

    #[test]
    fn corrupt_lines_are_skipped_not_fatal() {
        // Upstream returns null for this file; we deliberately keep it.
        let jsonl = format!("{}\n{{torn line", user_line("survives"));
        let d = digest_session(&jsonl).unwrap();
        assert_eq!(d.summary, "survives");
    }

    #[test]
    fn sessions_without_a_real_prompt_are_skipped() {
        assert_eq!(digest_session(""), None);
        // Messages but only local-command text → no summary → None.
        let jsonl = user_line("<bash-input>ls</bash-input>");
        assert_eq!(digest_session(&jsonl), None);
        // A summary candidate but zero countable messages → None.
        let jsonl = serde_json::json!({"type":"summary","message":"x"}).to_string();
        assert_eq!(digest_session(&jsonl), None);
    }

    #[test]
    fn summary_is_truncated_to_120_utf16_units() {
        let long = "a".repeat(300);
        let d = digest_session(&user_line(&long)).unwrap();
        assert_eq!(d.summary.len(), 120);
        // An astral char (2 UTF-16 units) straddling the boundary is dropped
        // whole, not split.
        let tricky = format!("{}𝄞end", "a".repeat(119));
        let d = digest_session(&user_line(&tricky)).unwrap();
        assert_eq!(d.summary, "a".repeat(119));
    }

    #[test]
    fn text_content_stops_growing_at_the_cap() {
        let chunk = "x".repeat(500);
        let lines: Vec<String> = (0..40).map(|_| user_line(&chunk)).collect();
        let d = digest_session(&lines.join("\n")).unwrap();
        // 15 full slices reach 7515 units; the 16th is appended because the
        // check happens before the append (upstream semantics), then the
        // counter passes 8000 and everything after is dropped.
        let units = d.text_content.encode_utf16().count();
        assert!(units >= 8000, "cap is checked before append: {units}");
        assert!(
            units <= 8000 + 501,
            "but only one slice may overshoot: {units}"
        );
    }

    proptest! {
        #[test]
        fn digest_never_panics(input in any::<String>()) {
            let _ = digest_session(&input);
        }

        #[test]
        fn digest_of_a_real_prompt_roundtrips(text in "[a-zA-Z0-9 ]{1,100}") {
            let d = digest_session(&user_line(&text)).unwrap();
            prop_assert_eq!(d.summary, text);
        }

        #[test]
        fn the_last_reply_names_the_model(
            models in prop::collection::vec("[a-z0-9-]{1,20}", 1..8),
        ) {
            let mut lines = vec![user_line("prompt")];
            lines.extend(models.iter().map(|m| reply(serde_json::json!({ "model": m }))));
            let d = digest_session(&lines.join("\n")).unwrap();
            prop_assert_eq!(d.model.as_deref(), models.last().map(String::as_str));
        }
    }

    /// An assistant entry as Claude Code 2.1 writes it, `message` replaced by
    /// `fields` merged over a text reply, plus `extra` top-level keys.
    fn assistant_line(message: serde_json::Value, extra: serde_json::Value) -> String {
        let mut entry = serde_json::json!({
            "type": "assistant",
            "message": { "role": "assistant", "content": [{ "type": "text", "text": "ok" }] },
        });
        if let (Some(target), Some(fields)) =
            (entry["message"].as_object_mut(), message.as_object())
        {
            target.extend(fields.clone());
        }
        if let (Some(target), Some(fields)) = (entry.as_object_mut(), extra.as_object()) {
            target.extend(fields.clone());
        }
        entry.to_string()
    }

    fn reply(message: serde_json::Value) -> String {
        assistant_line(message, serde_json::json!({}))
    }

    #[test]
    fn version_model_and_effort_come_from_the_last_entries_carrying_them() {
        let jsonl = [
            serde_json::json!({"type":"user","message":"hi","version":"2.1.290"}).to_string(),
            assistant_line(
                serde_json::json!({"model":"claude-sonnet-5"}),
                serde_json::json!({"version":"2.1.290","effort":"low","perTurnEffort":"low"}),
            ),
            assistant_line(
                serde_json::json!({"model":"claude-opus-5-5"}),
                serde_json::json!({"version":"2.1.291","effort":"medium","perTurnEffort":"high"}),
            ),
            serde_json::json!({"type":"system","subtype":"x","version":"2.1.294"}).to_string(),
        ]
        .join("\n");
        let d = digest_session(&jsonl).unwrap();
        assert_eq!(d.version.as_deref(), Some("2.1.294"));
        assert_eq!(d.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(d.effort.as_deref(), Some("medium"));
    }

    #[test]
    fn a_transcript_recording_no_effort_has_none() {
        let jsonl = [
            user_line("hi"),
            assistant_line(
                serde_json::json!({"model":"claude-opus-5"}),
                serde_json::json!({"version":"2.1.284"}),
            ),
        ]
        .join("\n");
        let d = digest_session(&jsonl).unwrap();
        assert_eq!(d.effort, None);
        assert_eq!(d.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(d.version.as_deref(), Some("2.1.284"));
    }

    #[test]
    fn effort_falls_back_to_the_per_turn_effort() {
        let jsonl = [
            user_line("hi"),
            assistant_line(
                serde_json::json!({}),
                serde_json::json!({"perTurnEffort":"high"}),
            ),
        ]
        .join("\n");
        assert_eq!(
            digest_session(&jsonl).unwrap().effort.as_deref(),
            Some("high")
        );
    }

    #[test]
    fn a_later_reply_without_effort_keeps_the_earlier_one() {
        let jsonl = [
            user_line("hi"),
            assistant_line(
                serde_json::json!({}),
                serde_json::json!({"effort":"medium"}),
            ),
            reply(serde_json::json!({"model":"claude-opus-5-5"})),
        ]
        .join("\n");
        assert_eq!(
            digest_session(&jsonl).unwrap().effort.as_deref(),
            Some("medium")
        );
    }

    #[test]
    fn sidechain_and_synthetic_replies_do_not_name_the_model() {
        let jsonl = [
            user_line("hi"),
            reply(serde_json::json!({"model":"claude-opus-5-5"})),
            assistant_line(
                serde_json::json!({"model":"claude-haiku-5"}),
                serde_json::json!({"isSidechain":true,"effort":"low"}),
            ),
            reply(serde_json::json!({"model":"<synthetic>"})),
        ]
        .join("\n");
        let d = digest_session(&jsonl).unwrap();
        assert_eq!(d.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(d.effort, None);
    }

    #[test]
    fn a_transcript_recording_none_of_them_has_none() {
        let d = digest_session(&user_line("hi")).unwrap();
        assert_eq!((d.version, d.model, d.effort), (None, None, None));
    }

    #[test]
    fn a_version_on_an_entry_that_is_no_message_is_ignored() {
        let jsonl = [
            serde_json::json!({"type":"user","message":"hi","version":"2.1.290"}).to_string(),
            serde_json::json!({"type":"summary","version":"9.9.9"}).to_string(),
        ]
        .join("\n");
        assert_eq!(
            digest_session(&jsonl).unwrap().version.as_deref(),
            Some("2.1.290")
        );
    }
}
