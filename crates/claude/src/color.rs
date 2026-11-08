//! The colours Claude Code's `/color` command accepts.
//!
//! A closed palette rather than a free string: the same names are what a
//! session's transcript records when its colour changes, so a writer (the
//! command termherd types) and a reader (the transcript) share one vocabulary.
//! It lives in the codec rather than in `core` because reading it back out of a
//! transcript is a codec job, and `core` depends on this crate, not the reverse.

/// One entry of `/color`'s palette, or `Default`, which resets the session to
/// Claude's own colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClaudeColor {
    Red,
    Blue,
    Green,
    Yellow,
    Purple,
    Orange,
    Pink,
    Cyan,
    Default,
}

impl ClaudeColor {
    /// Every value, in the order a picker should offer them.
    pub const ALL: [Self; 9] = [
        Self::Red,
        Self::Blue,
        Self::Green,
        Self::Yellow,
        Self::Purple,
        Self::Orange,
        Self::Pink,
        Self::Cyan,
        Self::Default,
    ];

    /// The word `/color` takes for this value.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Blue => "blue",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Purple => "purple",
            Self::Orange => "orange",
            Self::Pink => "pink",
            Self::Cyan => "cyan",
            Self::Default => "default",
        }
    }

    /// The value `name` spells, ignoring case and surrounding whitespace, or
    /// `None` for a word outside the palette.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.trim();
        Self::ALL
            .into_iter()
            .find(|color| color.name().eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_colour_reads_back_from_its_own_name() {
        for color in ClaudeColor::ALL {
            assert_eq!(ClaudeColor::from_name(color.name()), Some(color));
        }
    }

    #[test]
    fn the_palette_is_eight_colours_and_a_reset_with_distinct_names() {
        let names: HashSet<_> = ClaudeColor::ALL.iter().map(|c| c.name()).collect();
        assert_eq!(names.len(), 9);
        assert!(names.contains("default"));
    }

    #[test]
    fn a_name_is_read_regardless_of_case_and_padding() {
        assert_eq!(ClaudeColor::from_name(" Cyan "), Some(ClaudeColor::Cyan));
        assert_eq!(ClaudeColor::from_name("PURPLE"), Some(ClaudeColor::Purple));
    }

    #[test]
    fn a_word_outside_the_palette_is_refused() {
        for word in ["", "magenta", "red blue", "#ff0000", "re d"] {
            assert_eq!(ClaudeColor::from_name(word), None, "{word:?}");
        }
    }
}
