//! The one writer of `settings.json` options, shared by every surface that sets
//! one — this stdio server, the live bridge and the GUI settings panel — so the
//! rule that guards the user's file is stated once.
//!
//! The rule: a file that exists but is not a JSON object is never written
//! over. Setting one option onto an empty object in its place would discard
//! everything else the user wrote.

use std::path::Path;

use serde_json::{Map, Value};

use crate::SetError;

/// Why [`set_option_at`] wrote nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetAtError {
    /// The catalogue refused the id or the value: the caller's to fix.
    Refused(SetError),
    /// The file could not be read, parsed as an object, or written.
    File(String),
}

impl std::fmt::Display for SetAtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetAtError::Refused(error) => error.fmt(f),
            SetAtError::File(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for SetAtError {}

/// The JSON object at `path`; an empty one when the file does not exist. A
/// file that is unreadable or not a JSON object is an error.
pub fn read_object(path: &Path) -> Result<Value, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Value::Object(Map::new()));
        }
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    match serde_json::from_str::<Value>(&raw) {
        Ok(value) if value.is_object() => Ok(value),
        _ => Err(format!(
            "{} is not a valid JSON object; fix it first",
            path.display()
        )),
    }
}

/// Pretty-print `value` to `path`, creating its directory.
pub fn write(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| format!("could not encode {}: {e}", path.display()))?;
    std::fs::write(path, json + "\n")
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// Set one catalogue option in the file at `path`, leaving every other key as
/// found. Nothing is written on any refusal.
pub fn set_option_at(path: &Path, id: &str, value: &Value) -> Result<(), SetAtError> {
    let settings = read_object(path).map_err(SetAtError::File)?;
    let edited = crate::set_option(&settings, id, value).map_err(SetAtError::Refused)?;
    write(path, &edited).map_err(SetAtError::File)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_file(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("termherd-mcp-file-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        dir.join("settings.json")
    }

    #[test]
    fn a_missing_file_reads_as_empty_and_a_set_creates_it() {
        let path = temp_file("missing");
        assert_eq!(read_object(&path), Ok(json!({})));

        set_option_at(&path, "theme", &json!("light")).expect("written");
        assert_eq!(read_object(&path), Ok(json!({ "theme": "light" })));
    }

    #[test]
    fn a_file_that_is_not_an_object_is_never_written_over() {
        let path = temp_file("broken");
        std::fs::write(&path, "{ broken").expect("write");

        let result = set_option_at(&path, "theme", &json!("light"));

        assert!(matches!(result, Err(SetAtError::File(_))));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "{ broken");
    }

    #[test]
    fn a_refused_value_writes_nothing() {
        let path = temp_file("refused");

        let result = set_option_at(&path, "theme", &json!("mauve"));

        assert!(matches!(result, Err(SetAtError::Refused(_))));
        assert!(!path.exists());
    }

    #[test]
    fn other_keys_survive_a_set() {
        let path = temp_file("siblings");
        std::fs::write(&path, r#"{ "future": 1, "terminal": { "font_size": 14 } }"#)
            .expect("write");

        set_option_at(&path, "terminal.colors.scheme", &json!("gruvbox-light")).expect("written");

        assert_eq!(
            read_object(&path),
            Ok(json!({
                "future": 1,
                "terminal": { "font_size": 14, "colors": { "scheme": "gruvbox-light" } }
            }))
        );
    }
}
