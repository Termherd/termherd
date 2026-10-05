//! The settings panel's appearance controls: the chrome theme and the
//! terminal scheme, each applied the moment it is picked and then persisted.
//! The chrome follows on the next frame, since [`Shell::theme`] reads it every
//! frame; the terminals and the file follow through
//! [`super::live_settings::SettingsSink`].

use std::fmt;

use termherd_pty::Palette;

use super::Shell;
use crate::settings::{AppearanceChange, ColorSettings, ThemeChoice};

/// What the settings panel edits.
pub(super) struct Appearance {
    pub(super) theme: ThemeChoice,
    /// Kept whole, so a picked scheme still honours the user's explicit
    /// colour overrides from `settings.json`.
    pub(super) colors: ColorSettings,
}

/// One entry of the terminal-scheme picker: a built-in scheme by name, or
/// `None` for the default palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SchemeChoice(pub(crate) Option<&'static str>);

impl SchemeChoice {
    /// The default palette, then every named scheme.
    pub(super) fn all() -> Vec<Self> {
        std::iter::once(Self(None))
            .chain(Palette::SCHEMES.into_iter().map(|name| Self(Some(name))))
            .collect()
    }

    /// The entry matching a configured scheme name. `None` for a name no
    /// built-in answers to, so the picker shows no selection rather than a
    /// wrong one.
    pub(super) fn of(scheme: Option<&str>) -> Option<Self> {
        match scheme {
            None => Some(Self(None)),
            Some(name) => Palette::SCHEMES
                .into_iter()
                .find(|known| *known == name)
                .map(|known| Self(Some(known))),
        }
    }
}

/// `solarized-light` reads as `Solarized Light`, as the chrome themes do.
impl fmt::Display for SchemeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(name) = self.0 else {
            return f.write_str(crate::strings::SETTINGS_SCHEME_BUILTIN);
        };
        let words: Vec<String> = name
            .split('-')
            .map(|word| {
                let mut chars = word.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_uppercase().chain(chars).collect()
                })
            })
            .collect();
        f.write_str(&words.join(" "))
    }
}

impl Shell {
    /// Apply a chrome theme and persist it.
    pub(super) fn pick_theme(&mut self, theme: ThemeChoice) {
        self.appearance.theme = theme;
        self.settings_sink.persist(&AppearanceChange::Theme(theme));
    }

    /// Apply a terminal scheme to every session and persist it.
    pub(super) fn pick_scheme(&mut self, scheme: SchemeChoice) {
        let name = scheme.0.map(str::to_owned);
        self.appearance.colors.scheme.clone_from(&name);
        self.config.terminal_scheme = name;
        self.settings_sink
            .repaint(self.appearance.colors.to_palette());
        self.settings_sink.persist(&AppearanceChange::Scheme(
            self.appearance.colors.scheme.clone(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scheme_entry_reads_as_words_and_finds_itself() {
        let labels: Vec<String> = SchemeChoice::all()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            labels,
            [
                "Built-in",
                "Solarized Dark",
                "Solarized Light",
                "Gruvbox Dark",
                "Gruvbox Light"
            ]
        );
        for choice in SchemeChoice::all() {
            assert_eq!(SchemeChoice::of(choice.0), Some(choice));
        }
        assert_eq!(SchemeChoice::of(Some("no-such")), None);
    }
}
