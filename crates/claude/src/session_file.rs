//! Claude Code's per-process session file, `~/.claude/sessions/<pid>.json`.
//!
//! Claude Code writes one while it runs, naming it after its own process id.
//! It is the only place the peer name other Claude sessions address it by
//! (`ListAgents` / `SendMessage`) is written down. Reading the file is the
//! scan adapter's job; this module only decodes it.

/// What termherd takes from a session file: the process it describes, and the
/// two identities a peer needs. A field Claude Code did not write is `None`,
/// since an older CLI or a session still starting writes fewer of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFile {
    /// The Claude process id, as the file itself states it.
    pub pid: u32,
    /// The peer name (`name`), e.g. `knowledge-hub-35`.
    pub name: Option<String>,
    /// The Claude session id (`sessionId`), the one `--resume` takes.
    pub session_id: Option<String>,
}

/// Decode the session file read from `<pid>.json`.
///
/// `None` when the text is not a JSON object, or when the `pid` it states is
/// not `pid`: a file whose content contradicts its name describes some other
/// process, and attributing its name to this one would be worse than none.
pub fn parse(json: &str, pid: u32) -> Option<SessionFile> {
    let _ = (json, pid);
    None
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
            "status": "idle",
        })
        .to_string()
    }

    #[test]
    fn a_full_file_yields_the_peer_name_and_the_session_id() {
        assert_eq!(
            parse(&file(65524), 65524),
            Some(SessionFile {
                pid: 65524,
                name: Some("knowledge-hub-35".to_owned()),
                session_id: Some("7eff318b-ee38-49ad-9a44-75d81c946c02".to_owned()),
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
