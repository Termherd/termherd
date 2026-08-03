//! Where a clicked file goes: the configured editor command, or the OS
//! default handler.
//!
//! The two decisions below hang on the same field, which is why they live
//! together. Opening through the OS is "do whatever the association says", so a
//! path that *is* a program has to be refused — see
//! [`crate::paths::runs_on_open`] for how partial that refusal is. A configured
//! command consults no association, so the refusal has nothing left to protect
//! and is lifted for the same field that answers the handoff.

use std::path::{Path, PathBuf};

use super::*;
use crate::open::{OpenCommand, OpenTarget};

impl App {
    /// Record the configured editor command (or its absence), from settings.
    pub(super) fn load_open_command(&mut self, command: Option<OpenCommand>) -> Vec<Effect> {
        self.open = command;
        Vec::new()
    }

    /// Where this file opens: the configured command with its argv filled in,
    /// or the OS default handler.
    pub(super) fn open_target(
        &self,
        path: PathBuf,
        line: Option<u32>,
        col: Option<u32>,
    ) -> OpenTarget {
        match &self.open {
            Some(command) => command.resolve(&path, line, col),
            None => OpenTarget::SystemHandler { path },
        }
    }

    /// Whether this file must not be opened at all: only true when the handoff
    /// would go through the OS association *and* the association would run it.
    pub(super) fn refuses_to_open(&self, real: &Path) -> bool {
        self.open.is_none() && crate::paths::runs_on_open(real)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The command every test here configures.
    fn configured() -> OpenCommand {
        OpenCommand::parse("code -g {path}:{line}:{col}").expect("valid command")
    }

    #[test]
    fn no_command_hands_off_to_the_os_and_keeps_the_refusal() {
        let app = App::new();
        assert_eq!(
            app.open_target("/repo/f.rs".into(), Some(42), None),
            OpenTarget::SystemHandler {
                path: "/repo/f.rs".into()
            }
        );
        assert!(
            app.refuses_to_open(Path::new("/repo/payload.app")),
            "the association would run it, so the click must not"
        );
    }

    #[test]
    fn a_configured_command_takes_over_and_lifts_the_refusal() {
        let mut app = App::new();
        app.apply(Event::OpenCommandLoaded(Some(configured())));

        assert_eq!(
            app.open_target("/repo/f.rs".into(), Some(42), None),
            OpenTarget::Editor {
                program: "code".to_owned(),
                args: vec!["-g".to_owned(), "/repo/f.rs:42:1".to_owned()],
            }
        );
        // Nothing consults an association any more: the whole reason the
        // denylist existed is gone, on every host — including the Windows
        // table it never covered.
        assert!(!app.refuses_to_open(Path::new("/repo/payload.app")));
        assert!(!app.refuses_to_open(Path::new("C:/tools/build.exe")));
    }

    #[test]
    fn an_unparsable_command_falls_back_rather_than_half_applying() {
        // The shell hands `None` when `settings.json` configures a command it
        // could not parse. That must be indistinguishable from no command at
        // all — refusal included, since the OS handoff is back in the path.
        let mut app = App::new();
        app.apply(Event::OpenCommandLoaded(Some(configured())));
        app.apply(Event::OpenCommandLoaded(None));
        assert_eq!(
            app.open_target("/repo/f.rs".into(), None, None),
            OpenTarget::SystemHandler {
                path: "/repo/f.rs".into()
            }
        );
        assert!(app.refuses_to_open(Path::new("/repo/payload.app")));
    }
}
