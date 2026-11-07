//! The workspace written down, so a restart can replay it: every tab's pane
//! tree with the launch each pane resumes, and which tab was shown. Pure data;
//! where it is kept is the shell's.

use termherd_claude::color::ClaudeColor;

use crate::workspace::{Branch, Pane, SplitDir, Tab};

use super::*;

/// The tab strip as a restart replays it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    /// In strip order: every unparked tab before every parked one.
    pub tabs: Vec<TabLayout>,
    /// The index of the tab shown.
    pub active: usize,
}

/// One tab: its pane tree and what the user set on it.
#[derive(Debug, Clone, PartialEq)]
pub struct TabLayout {
    pub root: PaneLayout,
    /// The path from the root to the focused pane.
    pub focus: Vec<Branch>,
    pub custom_title: Option<String>,
    pub color: Option<ClaudeColor>,
    pub parked: bool,
}

/// A pane tree whose leaves are launches rather than live sessions.
#[derive(Debug, Clone, PartialEq)]
pub enum PaneLayout {
    Leaf(PaneLaunch),
    Split {
        dir: SplitDir,
        ratio: f32,
        a: Box<PaneLayout>,
        b: Box<PaneLayout>,
    },
}

/// What a pane runs again, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneLaunch {
    pub cwd: Option<String>,
    pub launch: Launch,
}

impl Layout {
    /// The layout with a minted id on every fresh Claude pane, so each one's
    /// transcript is known from its first keystroke. `core` mints none itself.
    #[must_use]
    pub fn with_fresh_ids(mut self, mut mint: impl FnMut() -> String) -> Self {
        for tab in &mut self.tabs {
            tab.root.each_leaf_mut(&mut |pane| {
                let launch = std::mem::replace(&mut pane.launch, Launch::Shell);
                pane.launch = launch.with_fresh_id(&mut mint);
            });
        }
        self
    }
}

impl PaneLayout {
    fn each_leaf_mut(&mut self, visit: &mut impl FnMut(&mut PaneLaunch)) {
        match self {
            PaneLayout::Leaf(pane) => visit(pane),
            PaneLayout::Split { a, b, .. } => {
                a.each_leaf_mut(visit);
                b.each_leaf_mut(visit);
            }
        }
    }

    fn first_leaf(&self) -> &PaneLaunch {
        match self {
            PaneLayout::Leaf(pane) => pane,
            PaneLayout::Split { a, .. } => a.first_leaf(),
        }
    }
}

impl App {
    /// The strip as it stands, each pane's launch resolved as a reopen would
    /// resolve it: a Claude conversation the scan has found resumes.
    #[must_use]
    pub fn layout(&self) -> Layout {
        let tabs = &self.workspace.tabs;
        Layout {
            tabs: tabs.iter().filter_map(|tab| self.tab_layout(tab)).collect(),
            active: tabs
                .iter()
                .take(self.workspace.active)
                .filter(|tab| self.tab_layout(tab).is_some())
                .count(),
        }
    }

    /// `tab` written down; `None` when a pane names no live session, which
    /// leaves nothing to relaunch it from.
    fn tab_layout(&self, tab: &Tab) -> Option<TabLayout> {
        Some(TabLayout {
            root: self.pane_layout(&tab.root)?,
            focus: tab.focus.clone(),
            custom_title: tab.custom_title.clone(),
            color: tab.color,
            parked: tab.is_parked(),
        })
    }

    fn pane_layout(&self, pane: &Pane) -> Option<PaneLayout> {
        Some(match pane {
            Pane::Leaf(id) => {
                let session = self.sessions.get(id)?;
                PaneLayout::Leaf(PaneLaunch {
                    cwd: session.cwd.clone(),
                    launch: self.reopen_launch(session),
                })
            }
            Pane::Split { dir, ratio, a, b } => PaneLayout::Split {
                dir: *dir,
                ratio: *ratio,
                a: Box::new(self.pane_layout(a)?),
                b: Box::new(self.pane_layout(b)?),
            },
        })
    }

    /// Open `layout`'s tabs after the ones already open, spawning one session
    /// per pane. The saved tab is shown only when nothing was open before,
    /// which [`Effect::TabShown`] then reports.
    pub(super) fn restore_layout(&mut self, layout: Layout) -> Vec<Effect> {
        let was_empty = self.workspace.tabs.is_empty();
        let mut effects = Vec::new();
        let mut shown = None;
        for (index, tab) in layout.tabs.into_iter().enumerate() {
            if let Some(anchor) = self.restore_tab(tab, &mut effects)
                && index == layout.active
            {
                shown = Some(anchor);
            }
        }
        self.retitle_tabs();
        if was_empty && !effects.is_empty() {
            if let Some(index) = shown.and_then(|anchor| self.workspace.tab_of(anchor)) {
                self.show_restored(index);
            }
            effects.push(Effect::TabShown);
        }
        effects
    }

    /// Open one saved tab, pushing the spawn of each of its panes, and return
    /// its first session. `None` when no session id is left to allocate.
    fn restore_tab(&mut self, tab: TabLayout, effects: &mut Vec<Effect>) -> Option<SessionId> {
        let first = tab.root.first_leaf();
        let title = self.tab_title(first.cwd.as_deref().unwrap_or_default(), &first.launch);
        let root = self.start_panes(tab.root, effects)?;
        let index = self.workspace.restore(root, tab.focus, title, tab.parked);
        if let Some(name) = tab.custom_title {
            self.workspace.rename_tab(index, &name);
        }
        let restored = self.workspace.tabs.get_mut(index)?;
        restored.color = tab.color;
        Some(restored.first_session())
    }

    fn start_panes(&mut self, pane: PaneLayout, effects: &mut Vec<Effect>) -> Option<Pane> {
        Some(match pane {
            PaneLayout::Leaf(pane) => {
                let id = self.sessions.allocate()?;
                effects.push(self.start_session(id, pane.cwd, pane.launch));
                Pane::Leaf(id)
            }
            PaneLayout::Split { dir, ratio, a, b } => Pane::Split {
                dir,
                ratio,
                a: Box::new(self.start_panes(*a, effects)?),
                b: Box::new(self.start_panes(*b, effects)?),
            },
        })
    }

    /// Show the restored tab at `index`. A parked one is shown as it is only
    /// when every tab is parked; otherwise it is unparked, as any tab shown is.
    fn show_restored(&mut self, index: usize) {
        if self.workspace.first_parked() == 0 {
            self.workspace.active = index;
        } else {
            self.workspace.activate(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testsupport::record;

    fn shell_in(cwd: &str) -> PaneLaunch {
        PaneLaunch {
            cwd: Some(cwd.into()),
            launch: Launch::Shell,
        }
    }

    fn leaf(cwd: &str) -> PaneLayout {
        PaneLayout::Leaf(shell_in(cwd))
    }

    fn tab(root: PaneLayout) -> TabLayout {
        TabLayout {
            root,
            focus: Vec::new(),
            custom_title: None,
            color: None,
            parked: false,
        }
    }

    fn split(dir: SplitDir, ratio: f32, a: PaneLayout, b: PaneLayout) -> PaneLayout {
        PaneLayout::Split {
            dir,
            ratio,
            a: Box::new(a),
            b: Box::new(b),
        }
    }

    fn open(app: &mut App, cwd: &str, launch: Launch) {
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some(cwd.into()),
            launch,
            title: "t".into(),
            placement: Placement::Foreground,
        }));
    }

    fn spawns(effects: &[Effect]) -> Vec<&SpawnSpec> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Spawn(spec) => Some(spec),
                _ => None,
            })
            .collect()
    }

    /// Two tabs: a vertical split at 0.3 focused on its right pane, renamed and
    /// coloured, then a lone shell, parked; the first tab shown.
    fn two_tab_layout() -> Layout {
        Layout {
            tabs: vec![
                TabLayout {
                    root: split(SplitDir::Vertical, 0.3, leaf("/a"), leaf("/b")),
                    focus: vec![Branch::B],
                    custom_title: Some("Prod deploy".into()),
                    color: Some(ClaudeColor::Green),
                    parked: false,
                },
                TabLayout {
                    parked: true,
                    ..tab(leaf("/c"))
                },
            ],
            active: 0,
        }
    }

    #[test]
    fn an_empty_strip_captures_no_tab() {
        assert_eq!(App::new().layout(), Layout::default());
    }

    #[test]
    fn a_capture_keeps_the_tree_its_ratios_focus_name_colour_and_park() {
        let mut app = App::new();
        open(&mut app, "/a", Launch::Shell);
        app.apply(Event::SplitFocused(SplitDir::Vertical));
        app.workspace.set_split_ratio(0, &[], 0.3);
        app.apply(Event::RenameTab {
            index: 0,
            title: "Prod deploy".into(),
        });
        app.workspace.tabs[0].color = Some(ClaudeColor::Green);
        open(&mut app, "/c", Launch::Shell);
        app.apply(Event::ToggleParkTab(1));
        app.apply(Event::ActivateTab(0));

        // The split inherits the cwd of the pane it split: `/a`, not `/b`.
        let mut expected = two_tab_layout();
        expected.tabs[0].root = split(SplitDir::Vertical, 0.3, leaf("/a"), leaf("/a"));
        assert_eq!(app.layout(), expected);
    }

    #[test]
    fn a_capture_reads_where_a_pane_is_now_not_where_it_started() {
        let mut app = App::new();
        open(&mut app, "/a", Launch::Shell);
        let session = app.workspace.focused_session().expect("open");
        app.apply(Event::SessionCwdChanged {
            session,
            cwd: "/a/sub".into(),
        });
        assert_eq!(app.layout().tabs[0].root, leaf("/a/sub"));
    }

    #[test]
    fn a_capture_resumes_a_claude_conversation_the_scan_found() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record("conv", "/p", "work")]));
        open(
            &mut app,
            "/p",
            Launch::Claude(ClaudeLaunch::Fresh(Some("conv".into()))),
        );
        let PaneLayout::Leaf(pane) = &app.layout().tabs[0].root else {
            panic!("a lone pane");
        };
        assert_eq!(
            pane.launch,
            Launch::Claude(ClaudeLaunch::Resume("conv".into()))
        );
    }

    #[test]
    fn a_capture_keeps_a_fresh_claude_nobody_wrote_to_fresh() {
        let mut app = App::new();
        open(
            &mut app,
            "/p",
            Launch::Claude(ClaudeLaunch::Fresh(Some("unwritten".into()))),
        );
        let PaneLayout::Leaf(pane) = &app.layout().tabs[0].root else {
            panic!("a lone pane");
        };
        assert_eq!(
            pane.launch,
            Launch::Claude(ClaudeLaunch::Fresh(Some("unwritten".into())))
        );
    }

    #[test]
    fn a_restore_spawns_one_pane_per_leaf_where_it_ran() {
        let mut app = App::new();
        let effects = app.apply(Event::RestoreLayout(two_tab_layout()));
        let cwds: Vec<_> = spawns(&effects)
            .iter()
            .map(|spec| spec.cwd.as_deref())
            .collect();
        assert_eq!(cwds, [Some("/a"), Some("/b"), Some("/c")]);
        for spec in spawns(&effects) {
            assert!(app.sessions.contains_key(&spec.session), "registered");
            assert_eq!(spec.launch, Launch::Shell);
        }
    }

    #[test]
    fn a_restore_rebuilds_the_strip_it_was_given() {
        let mut app = App::new();
        app.apply(Event::RestoreLayout(two_tab_layout()));
        assert_eq!(app.layout(), two_tab_layout());
        assert_eq!(app.workspace.tabs[0].display_title(), "Prod deploy");
        assert!(app.workspace.tabs[1].is_parked());
    }

    #[test]
    fn a_restore_shows_the_saved_tab_and_says_so() {
        let mut layout = two_tab_layout();
        layout.tabs[1].parked = false;
        layout.active = 1;
        let mut app = App::new();
        let effects = app.apply(Event::RestoreLayout(layout));
        assert_eq!(app.workspace.active, 1);
        assert!(matches!(effects.last(), Some(Effect::TabShown)));
    }

    #[test]
    fn a_restore_after_a_tab_was_opened_appends_and_keeps_it_shown() {
        let mut app = App::new();
        open(&mut app, "/first", Launch::Shell);
        let shown = app.workspace.shown_session();
        let mut layout = two_tab_layout();
        layout.tabs[1].parked = false;
        layout.active = 1;
        app.apply(Event::RestoreLayout(layout));
        assert_eq!(app.workspace.tabs.len(), 3);
        assert_eq!(app.workspace.shown_session(), shown);
    }

    #[test]
    fn an_empty_layout_restores_nothing() {
        let mut app = App::new();
        assert!(
            app.apply(Event::RestoreLayout(Layout::default()))
                .is_empty()
        );
        assert!(app.workspace.tabs.is_empty());
    }

    #[test]
    fn a_saved_active_index_past_the_strip_shows_the_first_tab() {
        let mut layout = two_tab_layout();
        layout.active = 9;
        let mut app = App::new();
        app.apply(Event::RestoreLayout(layout));
        assert_eq!(app.workspace.active, 0);
    }

    #[test]
    fn a_focus_path_that_misses_a_leaf_lands_on_the_first_pane() {
        let mut layout = two_tab_layout();
        layout.tabs[0].focus = vec![Branch::B, Branch::A];
        let mut app = App::new();
        app.apply(Event::RestoreLayout(layout));
        assert_eq!(app.workspace.tabs[0].focus, [Branch::A]);
    }

    #[test]
    fn a_ratio_out_of_range_is_brought_back_into_it() {
        for (saved, restored) in [(f32::NAN, 0.5), (0.0, 0.1), (7.0, 0.9)] {
            let layout = Layout {
                tabs: vec![tab(split(
                    SplitDir::Horizontal,
                    saved,
                    leaf("/a"),
                    leaf("/b"),
                ))],
                active: 0,
            };
            let mut app = App::new();
            app.apply(Event::RestoreLayout(layout));
            let PaneLayout::Split { ratio, .. } = app.layout().tabs[0].root else {
                panic!("a split");
            };
            assert_eq!(ratio, restored, "saved {saved}");
        }
    }

    #[test]
    fn a_parked_tab_saved_before_an_unparked_one_still_ends_the_strip() {
        let mut layout = two_tab_layout();
        layout.tabs.swap(0, 1);
        layout.active = 1;
        let mut app = App::new();
        app.apply(Event::RestoreLayout(layout));
        assert!(!app.workspace.tabs[0].is_parked());
        assert!(app.workspace.tabs[1].is_parked());
        assert_eq!(app.workspace.active, 0, "the shown tab follows its move");
    }

    #[test]
    fn fresh_ids_are_minted_for_fresh_claude_panes_only() {
        let fresh = PaneLayout::Leaf(PaneLaunch {
            cwd: None,
            launch: Launch::Claude(ClaudeLaunch::Fresh(None)),
        });
        let resumed = PaneLayout::Leaf(PaneLaunch {
            cwd: None,
            launch: Launch::Claude(ClaudeLaunch::Resume("kept".into())),
        });
        let layout = Layout {
            tabs: vec![
                tab(split(SplitDir::Vertical, 0.5, fresh.clone(), resumed)),
                tab(fresh),
                tab(leaf("/s")),
            ],
            active: 0,
        };
        let mut minted = 0;
        let layout = layout.with_fresh_ids(|| {
            minted += 1;
            format!("id-{minted}")
        });
        let launches: Vec<Launch> = layout
            .tabs
            .iter()
            .flat_map(|tab| leaves(&tab.root))
            .map(|pane| pane.launch.clone())
            .collect();
        assert_eq!(
            launches,
            [
                Launch::Claude(ClaudeLaunch::Fresh(Some("id-1".into()))),
                Launch::Claude(ClaudeLaunch::Resume("kept".into())),
                Launch::Claude(ClaudeLaunch::Fresh(Some("id-2".into()))),
                Launch::Shell,
            ]
        );
    }

    fn leaves(pane: &PaneLayout) -> Vec<&PaneLaunch> {
        match pane {
            PaneLayout::Leaf(launch) => vec![launch],
            PaneLayout::Split { a, b, .. } => {
                let mut out = leaves(a);
                out.extend(leaves(b));
                out
            }
        }
    }

    mod round_trip {
        use super::*;
        use proptest::prelude::*;

        fn pane() -> impl Strategy<Value = PaneLayout> {
            let leaf = (0u8..4).prop_map(|n| leaf(&format!("/p{n}")));
            leaf.prop_recursive(4, 16, 2, |inner| {
                (
                    prop_oneof![Just(SplitDir::Vertical), Just(SplitDir::Horizontal)],
                    // Ratios a restore keeps as they are.
                    (1u8..=9).prop_map(|tenths| f32::from(tenths) / 10.0),
                    inner.clone(),
                    inner,
                )
                    .prop_map(|(dir, ratio, a, b)| split(dir, ratio, a, b))
            })
        }

        /// A path from `root` to one of its leaves, picked by `seed`.
        fn path_to_a_leaf(root: &PaneLayout, mut seed: u64) -> Vec<Branch> {
            let mut path = Vec::new();
            let mut pane = root;
            while let PaneLayout::Split { a, b, .. } = pane {
                let (branch, next) = if seed.is_multiple_of(2) {
                    (Branch::A, a)
                } else {
                    (Branch::B, b)
                };
                seed /= 2;
                path.push(branch);
                pane = next;
            }
            path
        }

        fn layout() -> impl Strategy<Value = Layout> {
            prop::collection::vec((pane(), any::<u64>(), any::<bool>()), 1..5)
                .prop_flat_map(|tabs| {
                    let len = tabs.len();
                    let mut tabs: Vec<TabLayout> = tabs
                        .into_iter()
                        .map(|(root, seed, parked)| TabLayout {
                            focus: path_to_a_leaf(&root, seed),
                            root,
                            custom_title: None,
                            color: None,
                            parked,
                        })
                        .collect();
                    // A strip keeps its parked tabs last.
                    tabs.sort_by_key(|tab| tab.parked);
                    (Just(tabs), 0..len)
                })
                .prop_filter_map(
                    "a parked tab is shown only when all are",
                    |(tabs, active)| {
                        let shown_parked = tabs[active].parked;
                        let all_parked = tabs.iter().all(|tab| tab.parked);
                        (!shown_parked || all_parked).then_some(Layout { tabs, active })
                    },
                )
        }

        proptest! {
            #[test]
            fn a_restored_strip_captures_as_the_layout_it_was_given(layout in layout()) {
                let mut app = App::new();
                app.apply(Event::RestoreLayout(layout.clone()));
                prop_assert_eq!(app.layout(), layout);
            }
        }
    }
}
