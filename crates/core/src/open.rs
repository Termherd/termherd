//! How a file leaves termherd: the configured editor command, or the OS
//! default handler.
//!
//! Pure — the parse and the substitution live here, the spawn lives in the
//! shell. Two properties are structural rather than checked:
//!
//! - **The command is argv, never a shell line.** A configured string is split
//!   into words *before* any substitution, so a `{path}` carrying spaces,
//!   quotes or `&` lands in exactly one argument and can never add another.
//! - **A word is scanned once.** Parsing turns it into literal and slot
//!   segments; rendering walks those segments. A path that literally reads
//!   `{line}` is a literal on the way out, because nothing re-reads it.

use std::path::{Path, PathBuf};

/// The position substituted when the terminal printed none. A file opens at
/// its head either way; `1` is the value that stays well-formed in every
/// editor grammar, where an empty string breaks `code -g f.rs:{line}:{col}`
/// and an omitted argument breaks `vim +{line} f.rs`.
const NO_POSITION: u32 = 1;

/// What a template word can stand in for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// The resolved file, as the terminal's click reached it.
    Path,
    /// The 1-based line, or [`NO_POSITION`].
    Line,
    /// The 1-based column, or [`NO_POSITION`].
    Col,
}

impl Slot {
    /// The slot a `{name}` names, or `None` for a name nothing substitutes.
    fn named(name: &str) -> Option<Self> {
        match name {
            "path" => Some(Slot::Path),
            "line" => Some(Slot::Line),
            "col" => Some(Slot::Col),
            _ => None,
        }
    }
}

/// One piece of a parsed word: text to emit verbatim, or a value to fill in.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Slot(Slot),
}

/// One argument template, parsed into segments once.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Word(Vec<Segment>);

impl Word {
    /// Split a configured word into literal and slot segments, rejecting a
    /// placeholder nothing substitutes. The single scanner: what parsing reads
    /// here is exactly what rendering walks, so the two cannot disagree about
    /// where a placeholder starts.
    fn parse(word: &str) -> Result<Self, OpenCommandError> {
        let mut segments = Vec::new();
        let mut rest = word;
        while let Some(open) = rest.find('{') {
            let (literal, after) = rest.split_at(open);
            if !literal.is_empty() {
                segments.push(Segment::Literal(literal.to_owned()));
            }
            let after = &after['{'.len_utf8()..];
            let close =
                after
                    .find('}')
                    .ok_or_else(|| OpenCommandError::UnterminatedPlaceholder {
                        word: word.to_owned(),
                    })?;
            let (name, tail) = after.split_at(close);
            let slot = Slot::named(name).ok_or_else(|| OpenCommandError::UnknownPlaceholder {
                name: name.to_owned(),
            })?;
            segments.push(Segment::Slot(slot));
            rest = &tail['}'.len_utf8()..];
        }
        if !rest.is_empty() {
            segments.push(Segment::Literal(rest.to_owned()));
        }
        Ok(Self(segments))
    }

    /// Whether this word carries the file itself.
    fn takes_the_path(&self) -> bool {
        self.0.contains(&Segment::Slot(Slot::Path))
    }

    /// Render for one file. Literals are emitted as they were parsed, never
    /// re-read — which is why a filename that *contains* `{line}` stays one.
    fn render(&self, path: &Path, line: u32, col: u32) -> String {
        self.0.iter().fold(String::new(), |mut out, segment| {
            match segment {
                Segment::Literal(text) => out.push_str(text),
                Segment::Slot(Slot::Path) => out.push_str(&path.to_string_lossy()),
                Segment::Slot(Slot::Line) => out.push_str(&line.to_string()),
                Segment::Slot(Slot::Col) => out.push_str(&col.to_string()),
            }
            out
        })
    }
}

/// Why a configured `open.command` cannot be used. Each one degrades the whole
/// command (the shell warns and falls back to the OS handler) rather than
/// half-applying: a command that reaches the editor missing its file is worse
/// than one that never ran.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpenCommandError {
    /// No words at all — nothing to run.
    #[error("the open command is empty")]
    Empty,
    /// A placeholder in the program name would let whatever the terminal
    /// printed choose the executable. Refused, not substituted.
    #[error("the open command's program `{program}` must not contain a placeholder")]
    PlaceholderInProgram { program: String },
    /// A `{name}` nothing substitutes: passing it through verbatim would reach
    /// the editor as a filename.
    #[error("unknown placeholder `{{{name}}}` in the open command")]
    UnknownPlaceholder { name: String },
    /// A `{` with no closing `}`.
    #[error("unterminated placeholder in the open command word `{word}`")]
    UnterminatedPlaceholder { word: String },
    /// The command never receives the file it is supposed to open.
    #[error("the open command has no `{{path}}` placeholder")]
    NoPath,
}

/// A configured editor command, validated at parse time so rendering cannot
/// fail. Built from [`OpenCommand::parse`] (a whitespace-split line) or
/// [`OpenCommand::from_words`] (an explicit argv).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCommand {
    program: String,
    args: Vec<Word>,
}

/// Where a click hands a file off. Already resolved by `core`, so the shell
/// only spawns: either the user's command with its argv filled in, or the OS
/// default handler when none is configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenTarget {
    /// The configured command, substituted.
    Editor { program: String, args: Vec<String> },
    /// The OS default handler (`open` / `explorer` / `xdg-open`).
    SystemHandler { path: PathBuf },
}

impl OpenCommand {
    /// Parse a configured command line: split on whitespace, then validate.
    /// The split happens before substitution, which is what keeps a path with
    /// spaces one argument.
    ///
    /// # Errors
    /// See [`OpenCommandError`].
    pub fn parse(line: &str) -> Result<Self, OpenCommandError> {
        Self::from_words(line.split_whitespace().map(str::to_owned))
    }

    /// Build from an explicit argv — the form for a program path that carries
    /// spaces, which the split form cannot express.
    ///
    /// The program is taken as written: no `~` expansion, no shell lookup
    /// beyond the `PATH` the process already has. That is the price of never
    /// rendering this into a shell grammar, and the reason a bare `code` can
    /// fail under a desktop-launched app.
    ///
    /// # Errors
    /// See [`OpenCommandError`].
    pub fn from_words<I>(words: I) -> Result<Self, OpenCommandError>
    where
        I: IntoIterator<Item = String>,
    {
        let mut words = words.into_iter();
        let program = words.next().ok_or(OpenCommandError::Empty)?;
        // The program is checked, never substituted: a placeholder here would
        // let whatever the terminal printed pick the executable.
        if !Word::parse(&program)?.0.iter().all(is_literal) {
            return Err(OpenCommandError::PlaceholderInProgram { program });
        }
        let args = words
            .map(|word| Word::parse(&word))
            .collect::<Result<Vec<_>, _>>()?;
        if !args.iter().any(Word::takes_the_path) {
            return Err(OpenCommandError::NoPath);
        }
        Ok(Self { program, args })
    }

    /// Fill the templates in for one file. Infallible: every failure mode was
    /// settled at parse time. An absent line/column becomes [`NO_POSITION`].
    #[must_use]
    pub fn resolve(&self, path: &Path, line: Option<u32>, col: Option<u32>) -> OpenTarget {
        let (line, col) = (line.unwrap_or(NO_POSITION), col.unwrap_or(NO_POSITION));
        OpenTarget::Editor {
            program: self.program.clone(),
            args: self
                .args
                .iter()
                .map(|word| word.render(path, line, col))
                .collect(),
        }
    }
}

/// Whether a segment is plain text — the shape the program name must have.
fn is_literal(segment: &Segment) -> bool {
    matches!(segment, Segment::Literal(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The `(program, args)` a command renders for a file, or a failed test.
    fn rendered(
        command: &OpenCommand,
        path: &str,
        line: Option<u32>,
        col: Option<u32>,
    ) -> (String, Vec<String>) {
        match command.resolve(Path::new(path), line, col) {
            OpenTarget::Editor { program, args } => (program, args),
            other => panic!("expected an editor target, got {other:?}"),
        }
    }

    #[test]
    fn a_line_splits_into_argv_and_substitutes_the_position() {
        let cmd = OpenCommand::parse("code -g {path}:{line}:{col}").expect("valid command");
        let (program, args) = rendered(&cmd, "/repo/src/main.rs", Some(42), Some(7));
        assert_eq!(program, "code");
        assert_eq!(args, vec!["-g", "/repo/src/main.rs:42:7"]);
    }

    #[test]
    fn an_absent_position_substitutes_one_in_every_grammar() {
        // The terminal printed a bare path. Both shapes must stay well-formed:
        // an empty string would give `code -g f.rs::`, an omitted argument
        // `vim + f.rs`.
        let vscode = OpenCommand::parse("code -g {path}:{line}:{col}").expect("valid command");
        assert_eq!(
            rendered(&vscode, "/repo/f.rs", None, None).1,
            vec!["-g", "/repo/f.rs:1:1"]
        );
        let vim = OpenCommand::parse("vim +{line} {path}").expect("valid command");
        assert_eq!(
            rendered(&vim, "/repo/f.rs", None, None).1,
            vec!["+1", "/repo/f.rs"]
        );
        // A line without a column is the common case — only the column defaults.
        assert_eq!(
            rendered(&vscode, "/repo/f.rs", Some(42), None).1,
            vec!["-g", "/repo/f.rs:42:1"]
        );
    }

    #[test]
    fn an_explicit_argv_is_taken_verbatim() {
        // The form that expresses a program path containing spaces, which the
        // split form cannot.
        let cmd = OpenCommand::from_words(
            ["/Applications/My Editor", "-g", "{path}"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect("valid command");
        let (program, args) = rendered(&cmd, "/repo/f.rs", None, None);
        assert_eq!(program, "/Applications/My Editor");
        assert_eq!(args, vec!["-g", "/repo/f.rs"]);
    }

    #[test]
    fn a_command_that_cannot_open_a_file_is_refused_at_parse() {
        assert_eq!(OpenCommand::parse("   "), Err(OpenCommandError::Empty));
        assert_eq!(OpenCommand::parse("code -g"), Err(OpenCommandError::NoPath));
        assert_eq!(
            OpenCommand::parse("code {file}"),
            Err(OpenCommandError::UnknownPlaceholder {
                name: "file".to_owned()
            })
        );
        assert_eq!(
            OpenCommand::parse("code {path"),
            Err(OpenCommandError::UnterminatedPlaceholder {
                word: "{path".to_owned()
            })
        );
    }

    #[test]
    fn a_placeholder_never_chooses_the_executable() {
        // What the terminal printed decides which file opens, never which
        // program runs. Refused at parse rather than substituted.
        assert_eq!(
            OpenCommand::parse("{path} -g {path}"),
            Err(OpenCommandError::PlaceholderInProgram {
                program: "{path}".to_owned()
            })
        );
    }

    #[test]
    fn a_rendered_value_is_never_rescanned() {
        // A file literally named `{line}.rs` renders as itself: parsing splits
        // the word into segments once, and rendering walks those segments.
        let cmd = OpenCommand::parse("code {path}").expect("valid command");
        assert_eq!(
            rendered(&cmd, "/repo/{line}.rs", Some(42), None).1,
            vec!["/repo/{line}.rs"]
        );
    }

    proptest! {
        /// The argv shape is decided by the template alone. Whatever the
        /// terminal printed — spaces, quotes, `&`, a newline — lands in exactly
        /// one argument, so a filename can never add or remove one. This is the
        /// property the `cmd /C start` injection would have violated.
        #[test]
        fn a_path_fills_one_argument_whatever_it_contains(
            path in r#"[^\x00]{1,40}"#,
            line in proptest::option::of(1u32..10_000),
            col in proptest::option::of(1u32..10_000),
        ) {
            let cmd = OpenCommand::parse("code -g {path}:{line}:{col}").expect("valid command");
            let (program, args) = match cmd.resolve(Path::new(&path), line, col) {
                OpenTarget::Editor { program, args } => (program, args),
                other => return Err(TestCaseError::fail(format!("expected an editor, got {other:?}"))),
            };
            prop_assert_eq!(program, "code");
            prop_assert_eq!(args.len(), 2, "the template decides the argument count");
            prop_assert!(args[1].contains(&path), "the path reaches the editor intact");
            prop_assert_eq!(
                &args[1],
                &
                format!(
                    "{path}:{}:{}",
                    line.unwrap_or(NO_POSITION),
                    col.unwrap_or(NO_POSITION)
                )
            );
        }
    }
}
