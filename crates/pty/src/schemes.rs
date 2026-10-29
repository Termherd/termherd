//! The built-in `terminal.colors.scheme` presets, each named like the chrome
//! theme it matches. Their licences are in `THIRD_PARTY_NOTICES.md`, which a
//! new scheme owes a row.

use crate::grid::Palette;

impl Palette {
    /// Every name [`Palette::named`] resolves, in picker order.
    pub fn schemes() -> impl Iterator<Item = &'static str> {
        NAMED.iter().map(|(name, _)| *name)
    }

    /// A built-in scheme by its settings name, or `None` when unknown.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        NAMED
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, palette)| palette.clone())
    }

    const fn with_cursor(self, cursor: u32) -> Self {
        Self {
            cursor: hex(cursor),
            ..self
        }
    }
}

/// A scheme whose cursor sits on its foreground.
const fn scheme(foreground: u32, background: u32, ansi: &[u32; 16]) -> Palette {
    let mut rgb = [[0; 3]; 16];
    let mut i = 0;
    while i < 16 {
        rgb[i] = hex(ansi[i]);
        i += 1;
    }
    Palette {
        foreground: hex(foreground),
        background: hex(background),
        cursor: hex(foreground),
        ansi: rgb,
    }
}

/// Split a `0xrrggbb` literal into RGB.
const fn hex(c: u32) -> [u8; 3] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8]
}

static NAMED: [(&str, Palette); 18] = [
    (
        "solarized-dark",
        scheme(0x839496, 0x002b36, &SOLARIZED_ANSI),
    ),
    (
        "solarized-light",
        scheme(0x657b83, 0xfdf6e3, &SOLARIZED_ANSI),
    ),
    (
        "gruvbox-dark",
        scheme(0xebdbb2, 0x282828, &GRUVBOX_DARK_ANSI),
    ),
    (
        "gruvbox-light",
        scheme(0x3c3836, 0xfbf1c7, &GRUVBOX_LIGHT_ANSI),
    ),
    ("dracula", scheme(0xf8f8f2, 0x282a36, &DRACULA_ANSI)),
    ("nord", scheme(0xd8dee9, 0x2e3440, &NORD_ANSI)),
    (
        "catppuccin-latte",
        scheme(0x4c4f69, 0xeff1f5, &CATPPUCCIN_LATTE_ANSI).with_cursor(0xdc8a78),
    ),
    (
        "catppuccin-frappe",
        scheme(0xc6d0f5, 0x303446, &CATPPUCCIN_FRAPPE_ANSI).with_cursor(0xf2d5cf),
    ),
    (
        "catppuccin-macchiato",
        scheme(0xcad3f5, 0x24273a, &CATPPUCCIN_MACCHIATO_ANSI).with_cursor(0xf4dbd6),
    ),
    (
        "catppuccin-mocha",
        scheme(0xcdd6f4, 0x1e1e2e, &CATPPUCCIN_MOCHA_ANSI).with_cursor(0xf5e0dc),
    ),
    ("tokyo-night", scheme(0xa9b1d6, 0x1a1b26, &TOKYO_NIGHT_ANSI)),
    (
        "tokyo-night-storm",
        scheme(0xa9b1d6, 0x24283b, &TOKYO_NIGHT_STORM_ANSI),
    ),
    (
        "tokyo-night-light",
        scheme(0x343b58, 0xd6d8df, &TOKYO_NIGHT_LIGHT_ANSI).with_cursor(0x707280),
    ),
    (
        "kanagawa-wave",
        scheme(0xdcd7ba, 0x1f1f28, &KANAGAWA_WAVE_ANSI),
    ),
    (
        "kanagawa-dragon",
        scheme(0xc5c9c5, 0x181616, &KANAGAWA_DRAGON_ANSI),
    ),
    (
        "moonfly",
        scheme(0xbdbdbd, 0x080808, &MOONFLY_ANSI).with_cursor(0x8e8e8e),
    ),
    (
        "nightfly",
        scheme(0xbdc1c6, 0x011627, &NIGHTFLY_ANSI).with_cursor(0x9ca1aa),
    ),
    (
        "oxocarbon",
        scheme(0xffffff, 0x1b1b1b, &OXOCARBON_ANSI).with_cursor(0x78a9ff),
    ),
];

/// Solarized accents (shared by the dark and light variants, which differ
/// only in the base tones the fg/bg pick).
const SOLARIZED_ANSI: [u32; 16] = [
    0x073642, // base02
    0xdc322f, // red
    0x859900, // green
    0xb58900, // yellow
    0x268bd2, // blue
    0xd33682, // magenta
    0x2aa198, // cyan
    0xeee8d5, // base2
    0x002b36, // base03
    0xcb4b16, // orange
    0x586e75, // base01
    0x657b83, // base00
    0x839496, // base0
    0x6c71c4, // violet
    0x93a1a1, // base1
    0xfdf6e3, // base3
];

const GRUVBOX_DARK_ANSI: [u32; 16] = [
    0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984, 0x928374,
    0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
];

const GRUVBOX_LIGHT_ANSI: [u32; 16] = [
    0xfbf1c7, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0x7c6f64, 0x928374,
    0x9d0006, 0x79740e, 0xb57614, 0x076678, 0x8f3f71, 0x427b58, 0x3c3836,
];

const DRACULA_ANSI: [u32; 16] = [
    0x000000, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xbbbbbb, 0x555555,
    0xff5555, 0x50fa7b, 0xf1fa8c, 0xcaa9fa, 0xff79c6, 0x8be9fd, 0xffffff,
];

const NORD_ANSI: [u32; 16] = [
    0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0, 0x4c566a,
    0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
];

const CATPPUCCIN_LATTE_ANSI: [u32; 16] = [
    0x5c5f77, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xacb0be, 0x6c6f85,
    0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xbcc0cc,
];

const CATPPUCCIN_FRAPPE_ANSI: [u32; 16] = [
    0x51576d, 0xe78284, 0xa6d189, 0xe5c890, 0x8caaee, 0xf4b8e4, 0x81c8be, 0xb5bfe2, 0x626880,
    0xe78284, 0xa6d189, 0xe5c890, 0x8caaee, 0xf4b8e4, 0x81c8be, 0xa5adce,
];

const CATPPUCCIN_MACCHIATO_ANSI: [u32; 16] = [
    0x494d64, 0xed8796, 0xa6da95, 0xeed49f, 0x8aadf4, 0xf5bde6, 0x8bd5ca, 0xb8c0e0, 0x5b6078,
    0xed8796, 0xa6da95, 0xeed49f, 0x8aadf4, 0xf5bde6, 0x8bd5ca, 0xa5adcb,
];

const CATPPUCCIN_MOCHA_ANSI: [u32; 16] = [
    0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, 0x585b70,
    0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
];

const TOKYO_NIGHT_ANSI: [u32; 16] = [
    0x32344a, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xad8ee6, 0x449dab, 0x787c99, 0x444b6a,
    0xff7a93, 0xb9f27c, 0xff9e64, 0x7da6ff, 0xbb9af7, 0x0db9d7, 0xacb0d0,
];

const TOKYO_NIGHT_STORM_ANSI: [u32; 16] = [
    0x32344a, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xad8ee6, 0x449dab, 0x9699a8, 0x444b6a,
    0xff7a93, 0xb9f27c, 0xff9e64, 0x7da6ff, 0xbb9af7, 0x0db9d7, 0xacb0d0,
];

const TOKYO_NIGHT_LIGHT_ANSI: [u32; 16] = [
    0x343b58, 0xc24242, 0x41a6b5, 0x8f5e15, 0x2959aa, 0x7b43ba, 0x006c86, 0x707280, 0x343b58,
    0xc24242, 0x41a6b5, 0x8f5e15, 0x2959aa, 0x7b43ba, 0x006c86, 0x707280,
];

const KANAGAWA_WAVE_ANSI: [u32; 16] = [
    0x090618, 0xc34043, 0x76946a, 0xc0a36e, 0x7e9cd8, 0x957fb8, 0x6a9589, 0xc8c093, 0x727169,
    0xe82424, 0x98bb6c, 0xe6c384, 0x7fb4ca, 0x938aa9, 0x7aa89f, 0xdcd7ba,
];

const KANAGAWA_DRAGON_ANSI: [u32; 16] = [
    0x0d0c0c, 0xc4746e, 0x8a9a7b, 0xc4b28a, 0x8ba4b0, 0xa292a3, 0x8ea4a2, 0xc8c093, 0xa6a69c,
    0xe46876, 0x87a987, 0xe6c384, 0x7fb4ca, 0x938aa9, 0x7aa89f, 0xc5c9c5,
];

const MOONFLY_ANSI: [u32; 16] = [
    0x323437, 0xff5454, 0x8cc85f, 0xe3c78a, 0x80a0ff, 0xcf87e8, 0x79dac8, 0xc6c6c6, 0x949494,
    0xff5189, 0x36c692, 0xc6c684, 0x74b2ff, 0xae81ff, 0x85dc85, 0xe4e4e4,
];

const NIGHTFLY_ANSI: [u32; 16] = [
    0x1d3b53, 0xfc514e, 0xa1cd5e, 0xe3d18a, 0x82aaff, 0xc792ea, 0x7fdbca, 0xa1aab8, 0x7c8f8f,
    0xff5874, 0x21c7a8, 0xecc48d, 0x82aaff, 0xae81ff, 0x7fdbca, 0xd6deeb,
];

const OXOCARBON_ANSI: [u32; 16] = [
    0x161616, 0xee5396, 0x42be65, 0xff7eb6, 0x33b1ff, 0xbe95ff, 0x3ddbd9, 0xffffff, 0x525252,
    0xee5396, 0x42be65, 0xff7eb6, 0x33b1ff, 0xbe95ff, 0x3ddbd9, 0xffffff,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_schemes_resolve_and_unknown_is_none() {
        // Two darks, two lights — fg/bg from each published specification.
        let sd = Palette::named("solarized-dark").expect("known scheme");
        assert_eq!(sd.background, [0x00, 0x2b, 0x36]);
        assert_eq!(sd.foreground, [0x83, 0x94, 0x96]);
        let sl = Palette::named("solarized-light").expect("known scheme");
        assert_eq!(sl.background, [0xfd, 0xf6, 0xe3]);
        assert_eq!(sl.ansi, sd.ansi, "solarized variants share their accents");
        let gd = Palette::named("gruvbox-dark").expect("known scheme");
        assert_eq!(gd.background, [0x28, 0x28, 0x28]);
        let gl = Palette::named("gruvbox-light").expect("known scheme");
        assert_eq!(gl.background, [0xfb, 0xf1, 0xc7]);
        assert_ne!(gl.ansi, gd.ansi, "gruvbox brights differ per variant");
        for p in [&sd, &sl, &gd, &gl] {
            assert_eq!(p.cursor, p.foreground);
        }
        assert_eq!(Palette::named("no-such-scheme"), None);
    }

    /// Upstream values, one scheme with a published cursor and one without.
    #[test]
    fn imported_schemes_keep_their_published_values() {
        let mocha = Palette::named("catppuccin-mocha").expect("known scheme");
        assert_eq!(mocha.background, [0x1e, 0x1e, 0x2e]);
        assert_eq!(mocha.foreground, [0xcd, 0xd6, 0xf4]);
        assert_eq!(mocha.cursor, [0xf5, 0xe0, 0xdc]);
        assert_eq!(mocha.ansi[1], [0xf3, 0x8b, 0xa8], "normal red");
        let dracula = Palette::named("dracula").expect("known scheme");
        assert_eq!(dracula.background, [0x28, 0x2a, 0x36]);
        assert_eq!(dracula.cursor, dracula.foreground);
        assert_eq!(dracula.ansi[12], [0xca, 0xa9, 0xfa], "bright blue");
    }

    #[test]
    fn scheme_names_are_unique_kebab_case() {
        let names: Vec<_> = Palette::schemes().collect();
        for (i, name) in names.iter().enumerate() {
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{name} is not kebab-case"
            );
            assert!(!names[..i].contains(name), "{name} is listed twice");
        }
    }

    /// The licences ride with the binary only if each scheme has its row.
    #[test]
    fn every_scheme_is_credited_in_the_third_party_notices() {
        let credited: Vec<&str> = include_str!("../../../THIRD_PARTY_NOTICES.md")
            .lines()
            .filter_map(|line| line.strip_prefix('|')?.split('|').next())
            .flat_map(|cell| cell.split(','))
            .map(|name| name.trim().trim_matches('`'))
            .collect();
        for name in Palette::schemes() {
            assert!(
                credited.contains(&name),
                "{name} has no row in THIRD_PARTY_NOTICES.md"
            );
        }
    }

    #[test]
    fn every_listed_scheme_resolves() {
        for name in Palette::schemes() {
            assert!(
                Palette::named(name).is_some(),
                "{name} is listed but unknown"
            );
        }
    }
}
