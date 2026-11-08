//! Which name a session goes by. Every place that shows one — a tab chip, a
//! sidebar row — asks [`resolve`], so no two surfaces can rank the sources
//! differently.

/// Where a session's name can come from, highest precedence first. A blank
/// source counts as absent, so an empty title never hides a real one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TitleSources<'a> {
    /// A name the session was given on purpose: one kept for it in
    /// termherd's own metadata, else Claude's `/rename`.
    pub named: Option<&'a str>,
    /// The title the running program last reported over OSC.
    pub live: Option<&'a str>,
    /// What the transcript says the session is about: Claude's AI title,
    /// else its first prompt.
    pub described: Option<&'a str>,
    /// The label the session started with.
    pub launch: &'a str,
}

/// The name a session goes by, as [`TitleSources`] ranks them.
#[must_use]
pub fn resolve<'a>(sources: &TitleSources<'a>) -> &'a str {
    [sources.named, sources.live, sources.described]
        .into_iter()
        .flatten()
        .find(|title| !is_blank(title))
        .unwrap_or(sources.launch)
}

/// The first non-blank title in `titles`, in order — how one tier of
/// [`TitleSources`] picks between two candidates of its own.
#[must_use]
pub fn first_present<'a>(titles: impl IntoIterator<Item = Option<&'a str>>) -> Option<&'a str> {
    titles.into_iter().flatten().find(|title| !is_blank(title))
}

fn is_blank(title: &str) -> bool {
    title.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_named_session_outranks_every_other_source() {
        let sources = TitleSources {
            named: Some("auth refactor"),
            live: Some("Thinking"),
            described: Some("fix the login"),
            launch: "repo",
        };
        assert_eq!(resolve(&sources), "auth refactor");
    }

    #[test]
    fn a_live_title_outranks_the_description_and_the_launch_label() {
        let sources = TitleSources {
            named: None,
            live: Some("Thinking"),
            described: Some("fix the login"),
            launch: "repo",
        };
        assert_eq!(resolve(&sources), "Thinking");
    }

    #[test]
    fn the_description_outranks_the_launch_label() {
        let sources = TitleSources {
            described: Some("fix the login"),
            launch: "repo",
            ..TitleSources::default()
        };
        assert_eq!(resolve(&sources), "fix the login");
    }

    #[test]
    fn with_no_other_source_the_launch_label_stands() {
        let sources = TitleSources {
            launch: "repo",
            ..TitleSources::default()
        };
        assert_eq!(resolve(&sources), "repo");
    }

    #[test]
    fn a_blank_source_is_skipped_rather_than_shown() {
        let sources = TitleSources {
            named: Some("   "),
            live: Some(""),
            described: Some("fix the login"),
            launch: "repo",
        };
        assert_eq!(resolve(&sources), "fix the login");
    }

    #[test]
    fn first_present_skips_blank_candidates() {
        assert_eq!(first_present([None, Some(" "), Some("ai")]), Some("ai"));
        assert_eq!(first_present([None, Some("")]), None);
    }

    fn source() -> impl Strategy<Value = Option<String>> {
        prop_oneof![
            Just(None),
            Just(Some(String::new())),
            Just(Some("  ".to_owned())),
            "[a-z]{1,8}".prop_map(Some),
        ]
    }

    fn present(source: Option<&String>) -> Option<&str> {
        source.map(String::as_str).filter(|s| !s.trim().is_empty())
    }

    proptest! {
        /// The answer is always the highest-ranked present source, whatever
        /// combination of sources is present — never a lower one, never a
        /// blank.
        #[test]
        fn resolve_picks_the_highest_ranked_present_source(
            named in source(),
            live in source(),
            described in source(),
            launch in "[a-z]{0,8}",
        ) {
            let sources = TitleSources {
                named: named.as_deref(),
                live: live.as_deref(),
                described: described.as_deref(),
                launch: &launch,
            };
            let expected = present(named.as_ref())
                .or(present(live.as_ref()))
                .or(present(described.as_ref()))
                .unwrap_or(&launch);
            prop_assert_eq!(resolve(&sources), expected);
        }
    }
}
