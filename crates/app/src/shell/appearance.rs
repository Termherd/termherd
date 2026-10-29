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
#[derive(Default)]
pub(super) struct Appearance {
    pub(super) theme: ThemeChoice,
    /// Kept whole, so a picked scheme still honours the user's explicit
    /// colour overrides from `settings.json`.
    pub(super) colors: ColorSettings,
}

/// One entry of the terminal-scheme picker: the scheme sharing a chrome
/// theme's name, or `None` for the default palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SchemeChoice(pub(crate) Option<ThemeChoice>);

impl SchemeChoice {
    /// The default palette, then every theme that has a scheme.
    pub(super) fn all() -> Vec<Self> {
        let paired = ThemeChoice::ALL
            .into_iter()
            .filter(|theme| Palette::named(theme.settings_name()).is_some());
        std::iter::once(Self(None))
            .chain(paired.map(|theme| Self(Some(theme))))
            .collect()
    }

    /// The entry matching a configured scheme name. `None` for a name no
    /// built-in answers to, so the picker shows no selection rather than a
    /// wrong one.
    pub(super) fn of(scheme: Option<&str>) -> Option<Self> {
        Self::all()
            .into_iter()
            .find(|choice| choice.name() == scheme)
    }

    fn name(self) -> Option<&'static str> {
        self.0.map(ThemeChoice::settings_name)
    }
}

impl fmt::Display for SchemeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(theme) => theme.fmt(f),
            None => f.write_str(crate::strings::SETTINGS_SCHEME_BUILTIN),
        }
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
        let name = scheme.name().map(str::to_owned);
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
    fn the_picker_offers_every_scheme_under_its_theme_label() {
        let choices = SchemeChoice::all();
        let names: Vec<_> = choices.iter().filter_map(|c| c.name()).collect();
        assert_eq!(names, Palette::schemes().collect::<Vec<_>>());
        let labels: Vec<String> = choices.iter().map(ToString::to_string).collect();
        assert_eq!(labels[0], "Built-in");
        assert!(
            labels.iter().any(|l| l == "Catppuccin Frappé"),
            "{labels:?}"
        );
        for choice in choices {
            assert_eq!(SchemeChoice::of(choice.name()), Some(choice));
        }
        assert_eq!(SchemeChoice::of(Some("no-such")), None);
    }
}
