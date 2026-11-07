//! Tab-strip persistence — the layout a restart replays, at
//! `~/.termherd/layout.json`. A file adapter owned by the shell, like
//! [`crate::window_config`]: `core` captures and restores a
//! [`termherd_core::Layout`] and never does I/O. The file plumbing lives in
//! [`crate::json_store`].

use serde::{Deserialize, Serialize};
use termherd_core::workspace::{Branch, SplitDir};
use termherd_core::{ClaudeColor, ClaudeLaunch, Launch, Layout, PaneLaunch, PaneLayout, TabLayout};

use crate::paths::DataDir;

/// Load the saved layout; any problem (no file, bad JSON) yields an empty one —
/// a lost layout must never block startup. An unparseable file is set aside and
/// reported, since the next quit would otherwise overwrite it.
#[must_use]
pub fn load(dir: &DataDir) -> (Layout, Option<crate::json_store::LoadProblem>) {
    let (file, problem) = crate::json_store::load_json_checked::<LayoutFile>(&dir.file(FILE));
    (file.into(), problem)
}

/// Persist `layout`. Failures are logged, never fatal.
pub fn save(dir: &DataDir, layout: &Layout) {
    crate::json_store::save_json(&dir.file(FILE), &LayoutFile::from(layout));
}

/// `layout.json` as written: core's [`Layout`] in words a reader can follow.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct LayoutFile {
    tabs: Vec<TabFile>,
    active: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct TabFile {
    root: PaneFile,
    #[serde(default)]
    focus: Vec<Side>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// A `/color` word; one this build does not know is dropped on load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(default)]
    parked: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum PaneFile {
    Shell {
        cwd: Option<String>,
    },
    /// A fresh conversation is saved without the id it ran under: the restore
    /// mints a new one, as reopening a closed tab does.
    Claude {
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resume: Option<String>,
    },
    Split {
        dir: Dir,
        ratio: f32,
        a: Box<PaneFile>,
        b: Box<PaneFile>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Dir {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Side {
    A,
    B,
}

impl From<&Layout> for LayoutFile {
    fn from(layout: &Layout) -> Self {
        Self {
            tabs: layout.tabs.iter().map(TabFile::from).collect(),
            active: layout.active,
        }
    }
}

impl From<LayoutFile> for Layout {
    fn from(file: LayoutFile) -> Self {
        Self {
            tabs: file.tabs.into_iter().map(TabLayout::from).collect(),
            active: file.active,
        }
    }
}

impl From<&TabLayout> for TabFile {
    fn from(tab: &TabLayout) -> Self {
        Self {
            root: PaneFile::from(&tab.root),
            focus: tab.focus.iter().map(|&branch| Side::from(branch)).collect(),
            name: tab.custom_title.clone(),
            color: tab.color.map(|color| color.name().to_owned()),
            parked: tab.parked,
        }
    }
}

impl From<TabFile> for TabLayout {
    fn from(tab: TabFile) -> Self {
        Self {
            root: tab.root.into(),
            focus: tab.focus.into_iter().map(Branch::from).collect(),
            custom_title: tab.name,
            color: tab.color.as_deref().and_then(ClaudeColor::from_name),
            parked: tab.parked,
        }
    }
}

impl From<&PaneLayout> for PaneFile {
    fn from(pane: &PaneLayout) -> Self {
        match pane {
            PaneLayout::Leaf(PaneLaunch { cwd, launch }) => {
                let cwd = cwd.clone();
                match launch {
                    Launch::Shell => Self::Shell { cwd },
                    Launch::Claude(claude) => Self::Claude {
                        cwd,
                        resume: claude_resume(claude),
                    },
                }
            }
            PaneLayout::Split { dir, ratio, a, b } => Self::Split {
                dir: Dir::from(*dir),
                ratio: *ratio,
                a: Box::new(Self::from(a.as_ref())),
                b: Box::new(Self::from(b.as_ref())),
            },
        }
    }
}

fn claude_resume(claude: &ClaudeLaunch) -> Option<String> {
    match claude {
        ClaudeLaunch::Resume(id) => Some(id.clone()),
        ClaudeLaunch::Fresh(_) => None,
    }
}

impl From<PaneFile> for PaneLayout {
    fn from(pane: PaneFile) -> Self {
        match pane {
            PaneFile::Shell { cwd } => Self::Leaf(PaneLaunch {
                cwd,
                launch: Launch::Shell,
            }),
            PaneFile::Claude { cwd, resume } => Self::Leaf(PaneLaunch {
                cwd,
                launch: Launch::Claude(
                    resume.map_or(ClaudeLaunch::Fresh(None), ClaudeLaunch::Resume),
                ),
            }),
            PaneFile::Split { dir, ratio, a, b } => Self::Split {
                dir: dir.into(),
                ratio,
                a: Box::new((*a).into()),
                b: Box::new((*b).into()),
            },
        }
    }
}

impl From<SplitDir> for Dir {
    fn from(dir: SplitDir) -> Self {
        match dir {
            SplitDir::Vertical => Self::Vertical,
            SplitDir::Horizontal => Self::Horizontal,
        }
    }
}

impl From<Dir> for SplitDir {
    fn from(dir: Dir) -> Self {
        match dir {
            Dir::Vertical => Self::Vertical,
            Dir::Horizontal => Self::Horizontal,
        }
    }
}

impl From<Branch> for Side {
    fn from(branch: Branch) -> Self {
        match branch {
            Branch::A => Self::A,
            Branch::B => Self::B,
        }
    }
}

impl From<Side> for Branch {
    fn from(side: Side) -> Self {
        match side {
            Side::A => Self::A,
            Side::B => Self::B,
        }
    }
}

/// The file's name within the data dir (PRD §7).
const FILE: &str = "layout.json";

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(cwd: &str, launch: Launch) -> PaneLayout {
        PaneLayout::Leaf(PaneLaunch {
            cwd: Some(cwd.into()),
            launch,
        })
    }

    fn layout(fresh: Launch) -> Layout {
        Layout {
            tabs: vec![
                TabLayout {
                    root: PaneLayout::Split {
                        dir: SplitDir::Horizontal,
                        ratio: 0.25,
                        a: Box::new(pane(
                            "/repo",
                            Launch::Claude(ClaudeLaunch::Resume("conv".into())),
                        )),
                        b: Box::new(pane("/repo/sub", Launch::Shell)),
                    },
                    focus: vec![Branch::B],
                    custom_title: Some("Prod deploy".into()),
                    color: Some(ClaudeColor::Purple),
                    parked: false,
                },
                TabLayout {
                    root: PaneLayout::Leaf(PaneLaunch {
                        cwd: None,
                        launch: fresh,
                    }),
                    focus: Vec::new(),
                    custom_title: None,
                    color: None,
                    parked: true,
                },
            ],
            active: 0,
        }
    }

    fn data_dir() -> (tempfile::TempDir, DataDir) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = DataDir::at(tmp.path());
        (tmp, dir)
    }

    #[test]
    fn a_saved_layout_loads_back_as_it_was() {
        let (_tmp, dir) = data_dir();
        let saved = layout(Launch::Claude(ClaudeLaunch::Fresh(None)));
        save(&dir, &saved);
        assert_eq!(load(&dir).0, saved);
    }

    #[test]
    fn a_fresh_claude_pane_is_saved_without_the_id_it_ran_under() {
        // The restore mints a new one: the old id may already name a transcript.
        let (_tmp, dir) = data_dir();
        save(
            &dir,
            &layout(Launch::Claude(ClaudeLaunch::Fresh(Some("old".into())))),
        );
        assert_eq!(
            load(&dir).0,
            layout(Launch::Claude(ClaudeLaunch::Fresh(None)))
        );
    }

    #[test]
    fn the_layout_is_written_to_its_documented_file() {
        let (_tmp, dir) = data_dir();
        save(&dir, &layout(Launch::Shell));
        let text = std::fs::read_to_string(dir.file(FILE)).expect("written");
        assert!(text.contains("\"Prod deploy\""), "{text}");
    }

    #[test]
    fn no_file_loads_an_empty_layout() {
        let (_tmp, dir) = data_dir();
        assert_eq!(load(&dir).0, Layout::default());
    }

    #[test]
    fn a_corrupt_file_loads_an_empty_layout() {
        let (_tmp, dir) = data_dir();
        std::fs::write(dir.file(FILE), "{ not json").expect("write");
        let (layout, problem) = load(&dir);
        assert_eq!(layout, Layout::default());
        let problem = problem.expect("a corrupt layout is reported");
        let kept = problem.kept_as.expect("and set aside");
        assert_eq!(
            std::fs::read_to_string(kept).expect("readable"),
            "{ not json",
            "the next quit cannot overwrite what was there"
        );
    }

    #[test]
    fn a_colour_this_build_does_not_know_drops_the_colour_not_the_tab() {
        let (_tmp, dir) = data_dir();
        save(&dir, &layout(Launch::Shell));
        let path = dir.file(FILE);
        let text = std::fs::read_to_string(&path).expect("written");
        std::fs::write(&path, text.replace("\"purple\"", "\"ultraviolet\"")).expect("write");
        let loaded = load(&dir).0;
        assert_eq!(loaded.tabs.len(), 2);
        assert_eq!(loaded.tabs[0].color, None);
    }
}
