//! Claude Code's per-process session file, `~/.claude/sessions/<pid>.json`.
//!
//! Claude Code writes one while it runs, naming it after its own process id.
//! It is the only place the peer name other Claude sessions address it by
//! (`ListAgents` / `SendMessage`) is written down. Reading the file is the
//! scan adapter's job; this module only decodes it.

/// What termherd takes from a session file: the process it describes, the
/// two identities a peer needs, and the Claude Code version it runs. A field Claude Code did not write is `None`,
/// since an older CLI or a session still starting writes fewer of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFile {
    /// The Claude process id, as the file itself states it.
    pub pid: u32,
    /// The peer name (`name`), e.g. `knowledge-hub-35`.
    pub name: Option<String>,
    /// The Claude session id (`sessionId`), the one `--resume` takes.
    pub session_id: Option<String>,
    /// When the process that wrote the file started (`procStart`), in `ps`'s
    /// `lstart` form and UTC, e.g. `Wed Oct  7 06:48:07 2026`. A file outlives
    /// a Claude that crashed, and its pid is then free for any process to
    /// reuse: this stamp is what tells the writer from its successor.
    pub proc_start: Option<String>,
    /// The Claude Code version the process runs (`version`), e.g. `2.1.294`.
    pub version: Option<String>,
}

/// Decode the session file read from `<pid>.json`.
///
/// `None` when the text is not a JSON object, or when the `pid` it states is
/// not `pid`: a file whose content contradicts its name describes some other
/// process, and attributing its name to this one would be worse than none.
pub fn parse(json: &str, pid: u32) -> Option<SessionFile> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let object = value.as_object()?;
    let stated = object.get("pid")?.as_u64()?;
    if stated != u64::from(pid) {
        return None;
    }
    let text = |key: &str| {
        object
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .map(str::to_owned)
    };
    Some(SessionFile {
        pid,
        name: text("name"),
        session_id: text("sessionId"),
        proc_start: text("procStart"),
        version: text("version"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    /// A file as Claude Code 2.1 writes it, trimmed to the fields that matter
    /// plus two it also writes, so the decoder is shown to ignore extras.
    fn file(pid: u32) -> String {
        json!({
            "pid": pid,
            "sessionId": "7eff318b-ee38-49ad-9a44-75d81c946c02",
            "cwd": "/work/knowledge-hub",
            "name": "knowledge-hub-35",
            "messagingSocketPath": "/tmp/cc-socks/65524.sock",
            "procStart": "Wed Oct  7 06:48:07 2026",
            "version": "2.1.294",
            "status": "idle",
        })
        .to_string()
    }

    #[test]
    fn a_full_file_yields_the_peer_name_the_session_id_and_the_version() {
        assert_eq!(
            parse(&file(65524), 65524),
            Some(SessionFile {
                pid: 65524,
                name: Some("knowledge-hub-35".to_owned()),
                session_id: Some("7eff318b-ee38-49ad-9a44-75d81c946c02".to_owned()),
                proc_start: Some("Wed Oct  7 06:48:07 2026".to_owned()),
                version: Some("2.1.294".to_owned()),
            })
        );
    }

    #[test]
    fn a_file_without_a_name_still_identifies_its_process() {
        let text = json!({ "pid": 4242, "sessionId": "abc" }).to_string();
        assert_eq!(
            parse(&text, 4242),
            Some(SessionFile {
                pid: 4242,
                name: None,
                session_id: Some("abc".to_owned()),
                proc_start: None,
                version: None,
            })
        );
    }

    #[test]
    fn a_blank_name_counts_as_no_name() {
        let text = json!({ "pid": 7, "name": "  ", "sessionId": "" }).to_string();
        assert_eq!(
            parse(&text, 7),
            Some(SessionFile {
                pid: 7,
                name: None,
                session_id: None,
                proc_start: None,
                version: None,
            })
        );
    }

    #[test]
    fn a_file_stating_another_pid_is_refused() {
        assert_eq!(parse(&file(65524), 65525), None);
    }

    #[test]
    fn a_file_with_no_pid_is_refused() {
        let text = json!({ "name": "knowledge-hub-35" }).to_string();
        assert_eq!(parse(&text, 65524), None);
    }

    #[test]
    fn text_that_is_not_a_json_object_is_refused() {
        assert_eq!(parse("not json", 1), None);
        assert_eq!(parse("[1, 2]", 1), None);
        assert_eq!(parse("", 1), None);
    }

    proptest! {
        #[test]
        fn decoding_arbitrary_text_never_panics(text in ".*", pid in any::<u32>()) {
            let _ = parse(&text, pid);
        }
    }
}
