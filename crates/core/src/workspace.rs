//! Pane tree + tabs — pure data, no I/O.
//!
//! The riskiest feature in v0 (tabs + splits + focus) lives here and is fully
//! testable headless. See `docs/ARCHITECTURE.md` §6.

use std::num::NonZeroU64;

use termherd_claude::color::ClaudeColor;

/// Stable session identifier. Non-zero so `Option<SessionId>` is niche-sized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub NonZeroU64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
    Horizontal,
    Vertical,
}

/// A spatial pane-focus move (FR6). Left/Right traverse `Vertical` splits (side
/// by side); Up/Down traverse `Horizontal` splits (stacked). Movement cycles
/// within its own axis: stepping past the last pane wraps to the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    /// The split orientation this move traverses, and the branch it heads
    /// toward: B is right/bottom, A is left/top.
    fn axis_toward(self) -> (SplitDir, Branch) {
        match self {
            Direction::Right => (SplitDir::Vertical, Branch::B),
            Direction::Left => (SplitDir::Vertical, Branch::A),
            Direction::Down => (SplitDir::Horizontal, Branch::B),
            Direction::Up => (SplitDir::Horizontal, Branch::A),
        }
    }
}

/// One side of a split, used to address a pane along the tree path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch {
    A,
    B,
}

impl Branch {
    fn opposite(self) -> Self {
        match self {
            Branch::A => Branch::B,
            Branch::B => Branch::A,
        }
    }
}

/// One step through an ordered cycle of tabs or panes, wrapping at both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Next,
    Prev,
}

impl Step {
    /// The index one step from `current` in a cycle of `len` items; `None`
    /// when the cycle is empty.
    fn index_after(self, current: usize, len: usize) -> Option<usize> {
        let offset = match self {
            Step::Next => 1,
            Step::Prev => len.checked_sub(1)?,
        };
        (current + offset).checked_rem(len)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pane {
    Leaf(SessionId),
    Split {
        dir: SplitDir,
        ratio: f32,
        a: Box<Pane>,
        b: Box<Pane>,
    },
}

#[derive(Debug, Clone)]
pub struct Tab {
    pub root: Pane,
    /// Path from the root to the focused leaf. An empty path means the root
    /// itself is the focused leaf.
    pub focus: Vec<Branch>,
    /// The derived title: the highest-ranked of the tab's [`TabTitles`], as
    /// [`crate::title::resolve`] ranks them. Read-only by convention — it is
    /// re-resolved whenever a source changes, so a write here would be lost
    /// on the next one. Shown only when there is no manual override (see
    /// [`Tab::custom_title`] / [`Tab::display_title`]).
    pub title: String,
    /// A user-set name that overrides the derived [`title`](Self::title) and is
    /// never clobbered by a later derived/OSC update — the manual rename wins.
    /// `None` means "use the derived title"; a rename to blank reverts to it.
    pub custom_title: Option<String>,
    /// The colour picked for this tab in termherd, for the panes Claude does
    /// not colour. `Some(Default)` is a pick of "none", which hides even the
    /// colour of a Claude run by hand in a shell pane; `None` was never picked.
    pub color: Option<ClaudeColor>,
    /// Every source [`Self::title`] is resolved from.
    titles: TabTitles,
    /// Out of the tab cycle, at the strip's end. Private: [`Workspace`] keeps
    /// every parked tab after every unparked one.
    parked: bool,
}

/// The sources a tab's derived title is resolved from, each kept apart so a
/// higher-ranked one coming or going re-exposes the one beneath it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TabTitles {
    named: Option<String>,
    live: Option<String>,
    described: Option<String>,
    launch: String,
}

impl Tab {
    /// A tab hosting `session` alone, titled after its launch.
    fn new(session: SessionId, title: String) -> Self {
        Self {
            root: Pane::Leaf(session),
            focus: Vec::new(),
            title: title.clone(),
            custom_title: None,
            color: None,
            titles: TabTitles {
                launch: title,
                ..TabTitles::default()
            },
            parked: false,
        }
    }

    /// Whether the tab is parked: shown as a compact chip after the unparked
    /// tabs, and skipped by the tab cycle. Its sessions keep running.
    #[must_use]
    pub fn is_parked(&self) -> bool {
        self.parked
    }

    /// Every session hosted by this tab, in left-to-right pane order. A tab
    /// without splits yields exactly one id; closing the tab kills them all.
    #[must_use]
    pub fn sessions(&self) -> Vec<SessionId> {
        let mut out = Vec::new();
        collect_leaves(&self.root, &mut out);
        out
    }

    /// Whether one of this tab's panes hosts `session`. Unlike
    /// [`Self::sessions`], it allocates nothing.
    #[must_use]
    pub fn hosts(&self, session: SessionId) -> bool {
        pane_hosts(&self.root, session)
    }

    /// The session in this tab's first (leftmost) pane — the one the tab is
    /// named after, and whose conversation it stands for.
    #[must_use]
    pub fn first_session(&self) -> SessionId {
        first_leaf(&self.root)
    }

    /// The session in this tab's focused pane, `None` if the focus path no
    /// longer lands on a leaf.
    #[must_use]
    pub fn focused_session(&self) -> Option<SessionId> {
        match navigate(&self.root, &self.focus)? {
            Pane::Leaf(s) => Some(*s),
            Pane::Split { .. } => None,
        }
    }

    /// Record the title the tab's program last reported over OSC.
    pub fn set_live_title(&mut self, title: impl Into<String>) {
        self.titles.live = Some(title.into());
        self.resolve_title();
    }

    /// Record the titles a scan and the metadata overlay give the tab's
    /// conversation: the name it was given, and what it is about. `None` for
    /// either when there is none, which re-exposes the sources beneath it.
    pub fn set_recorded_titles(&mut self, named: Option<&str>, described: Option<&str>) {
        if self.titles.named.as_deref() != named || self.titles.described.as_deref() != described {
            self.titles.named = named.map(str::to_owned);
            self.titles.described = described.map(str::to_owned);
            self.resolve_title();
        }
    }

    fn resolve_title(&mut self) {
        let titles = &self.titles;
        self.title = crate::title::resolve(&crate::title::TitleSources {
            named: titles.named.as_deref(),
            live: titles.live.as_deref(),
            described: titles.described.as_deref(),
            launch: &titles.launch,
        })
        .to_owned();
    }

    /// The title to display: the manual override when set, else the derived
    /// title. This is the single read every surface (tab chip, close prompt,
    /// hover card) should use so the override is honoured everywhere.
    #[must_use]
    pub fn display_title(&self) -> &str {
        self.custom_title.as_deref().unwrap_or(&self.title)
    }
}

fn pane_hosts(pane: &Pane, session: SessionId) -> bool {
    match pane {
        Pane::Leaf(leaf) => *leaf == session,
        Pane::Split { a, b, .. } => pane_hosts(a, session) || pane_hosts(b, session),
    }
}

fn collect_leaves(pane: &Pane, out: &mut Vec<SessionId>) {
    match pane {
        Pane::Leaf(session) => out.push(*session),
        Pane::Split { a, b, .. } => {
            collect_leaves(a, out);
            collect_leaves(b, out);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Workspace {
    pub tabs: Vec<Tab>,
    /// Index of the active tab. Meaningful only when `tabs` is non-empty.
    pub active: usize,
}

impl Workspace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a session as a new tab, made active.
    pub fn open(&mut self, session: SessionId, title: impl Into<String>) {
        self.active = self.append(session, title);
    }

    /// Open a session as a new tab after the unparked tabs without activating
    /// it, returning its index. When no other tab is unparked, the new tab is
    /// active all the same.
    pub fn append(&mut self, session: SessionId, title: impl Into<String>) -> usize {
        self.insert_tab(Tab::new(session, title.into()))
    }

    /// Place a new unparked tab before the parked group, returning its index.
    fn insert_tab(&mut self, tab: Tab) -> usize {
        let at = self.first_parked();
        self.tabs.insert(at, tab);
        // An active index at or past `at` named a parked tab, which only holds
        // while every tab is parked: the new tab takes over.
        if self.active >= at {
            self.active = at;
        }
        at
    }

    /// Index of the tab hosting `session`, if any. A session lives in exactly
    /// one tab (possibly inside a split), so this lets the shell re-focus an
    /// already-open session instead of launching a duplicate. Also a stable
    /// handle for state that must survive tab reordering — an inline rename
    /// anchors on it rather than a positional index, which a reorder or a
    /// sibling close would shift.
    #[must_use]
    pub fn tab_of(&self, session: SessionId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.hosts(session))
    }

    /// Park the tab at `index`, or unpark it when it is parked. `None`,
    /// changing nothing, for a missing tab.
    pub fn toggle_park(&mut self, index: usize) -> Option<()> {
        if self.tabs.get(index)?.parked {
            self.unpark(index).map(drop)
        } else {
            self.park(index)
        }
    }

    /// Split the focused leaf in the active tab, opening `new` on side B.
    /// Focus moves to the new pane. Returns `None` if there is no active tab
    /// or the focus path does not resolve to a leaf.
    pub fn split(&mut self, dir: SplitDir, new: SessionId) -> Option<()> {
        let tab = self.tabs.get_mut(self.active)?;
        let target = navigate_mut(&mut tab.root, &tab.focus)?;
        // The current pane must be a leaf — otherwise the focus invariant
        // is broken, which we surface as `None` rather than panicking.
        if !matches!(target, Pane::Leaf(_)) {
            return None;
        }
        // Take ownership of the existing leaf without panicking, then wrap
        // it into a new Split node with `new` on side B.
        let taken = std::mem::replace(target, Pane::Leaf(new));
        *target = Pane::Split {
            dir,
            ratio: 0.5,
            a: Box::new(taken),
            b: Box::new(Pane::Leaf(new)),
        };
        tab.focus.push(Branch::B);
        Some(())
    }

    /// Set the ratio of the split at `path` in `tab_index`, clamped so neither
    /// pane falls below ten percent. Returns `None` for an invalid tab or path.
    pub fn set_split_ratio(&mut self, tab_index: usize, path: &[Branch], ratio: f32) -> Option<()> {
        if !ratio.is_finite() {
            return None;
        }
        let tab = self.tabs.get_mut(tab_index)?;
        let Pane::Split { ratio: current, .. } = navigate_mut(&mut tab.root, path)? else {
            return None;
        };
        *current = kept_ratio(ratio);
        Some(())
    }

    /// Open a tab holding a whole pane tree, as a saved layout left it, and
    /// return its index: before the parked tabs, or at the strip's end when
    /// `parked`. A focus path that does not end on a leaf falls back to the
    /// first pane, and each ratio is brought into the range a drag keeps it in
    /// (a ratio that is not a number splits evenly). The active tab is left as
    /// it is unless every other tab is parked.
    pub fn restore(
        &mut self,
        root: Pane,
        focus: Vec<Branch>,
        title: impl Into<String>,
        parked: bool,
    ) -> usize {
        let root = with_kept_ratios(root);
        let focus = if matches!(navigate(&root, &focus), Some(Pane::Leaf(_))) {
            focus
        } else {
            first_leaf_path(&root)
        };
        let mut tab = Tab::new(first_leaf(&root), title.into());
        tab.root = root;
        tab.focus = focus;
        if parked {
            tab.parked = true;
            self.tabs.push(tab);
            self.tabs.len() - 1
        } else {
            self.insert_tab(tab)
        }
    }

    /// Record the OSC title `session` reported on the tab hosting it. Only the
    /// tab's first pane — the one it is named after — speaks for it: a shell
    /// split beside a Claude would otherwise retitle the Claude's tab with its
    /// prompt. Returns `None` if no tab hosts the session; a pane that is not
    /// first is hosted, so it answers `Some` and changes nothing.
    pub fn set_live_title(&mut self, session: SessionId, title: impl Into<String>) -> Option<()> {
        let tab = self
            .tabs
            .iter_mut()
            .find(|tab| tab.sessions().contains(&session))?;
        if tab.first_session() == session {
            tab.set_live_title(title);
        }
        Some(())
    }

    /// Give the tab at `index` a manual name that overrides its derived title.
    /// A blank name (empty or whitespace) clears the override, reverting to the
    /// derived title. So does a name that, once trimmed, equals the derived
    /// title: an override identical to the derived value is redundant, and
    /// storing it would silently *freeze* the title against later OSC/digest
    /// relabels — so the tab keeps tracking its derived title instead. Returns
    /// `None` (changing nothing) when `index` is out of range.
    pub fn rename_tab(&mut self, index: usize, name: &str) -> Option<()> {
        let tab = self.tabs.get_mut(index)?;
        let trimmed = name.trim();
        tab.custom_title = if trimmed.is_empty() || trimmed == tab.title.trim() {
            None
        } else {
            Some(trimmed.to_owned())
        };
        Some(())
    }

    /// Title of the tab hosting `session` — what the user sees for that
    /// session, used to name its desktop notification. `None` if no tab hosts
    /// it.
    #[must_use]
    pub fn session_title(&self, session: SessionId) -> Option<&str> {
        self.tabs
            .iter()
            .find(|tab| tab.sessions().contains(&session))
            .map(Tab::display_title)
    }

    /// Index of the first parked tab: the number of unparked tabs, since every
    /// parked tab sits after them. The strip's length when none is parked.
    #[must_use]
    pub fn first_parked(&self) -> usize {
        self.tabs
            .iter()
            .position(Tab::is_parked)
            .unwrap_or(self.tabs.len())
    }

    /// Park the unparked tab at `index`: it moves to the front of the parked
    /// group. When it was active, the active tab becomes the unparked one that
    /// slides into its slot, else the new last unparked tab; with none left it
    /// stays active. `None`, changing nothing, for a parked or missing tab.
    pub fn park(&mut self, index: usize) -> Option<()> {
        if self.tabs.get(index)?.parked {
            return None;
        }
        let front_of_parked = self.first_parked() - 1;
        self.relocate(index, front_of_parked, true);
        Some(())
    }

    /// Unpark the parked tab at `index`: it moves to the end of the unparked
    /// group, returning its new index. The active tab stays the same tab,
    /// unless every tab was parked: the unparked one then becomes active.
    /// `None`, changing nothing, for an unparked or missing tab.
    pub fn unpark(&mut self, index: usize) -> Option<usize> {
        if !self.tabs.get(index)?.parked {
            return None;
        }
        let end_of_unparked = self.first_parked();
        self.relocate(index, end_of_unparked, false);
        Some(end_of_unparked)
    }

    /// The first session of the tab shown, so the shown tab can be recognised
    /// again once the strip reorders. `None` with no tab open.
    #[must_use]
    pub fn shown_session(&self) -> Option<SessionId> {
        self.tabs.get(self.active).map(Tab::first_session)
    }

    /// Whether the tab shown hosts `session`.
    #[must_use]
    pub fn shows(&self, session: SessionId) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|tab| tab.hosts(session))
    }

    /// Make the tab at `index` active (FR5), unparking it first when it is
    /// parked. Returns `None` if the index is out of range, leaving the active
    /// tab unchanged.
    pub fn activate(&mut self, index: usize) -> Option<()> {
        let tab = self.tabs.get(index)?;
        self.active = if tab.parked {
            self.unpark(index)?
        } else {
            index
        };
        Some(())
    }

    /// Move the tab at `from` so it lands at index `to` (FR5 drag-reorder),
    /// shifting the tabs in between. The tab takes the zone it lands in, parked
    /// or not; landing on the boundary keeps its own. The active tab is kept
    /// pointing at the *same* tab, whether it was the moved one or merely
    /// shifted by it, unless the move parked it. Returns `None` (no change) if
    /// either index is out of range; `from == to` is a successful no-op.
    pub fn move_tab(&mut self, from: usize, to: usize) -> Option<()> {
        // The predicate refuses an index off the strip, so it is the bounds
        // check too.
        let parked = self.lands_parked(from, to)?;
        if from != to {
            self.relocate(from, to, parked);
        }
        Some(())
    }

    /// The drop-zone rule: whether the tab at `from`, moved to index `to` of
    /// the resulting strip, ends up parked. Past the unparked tabs it parks,
    /// before them it unparks, and on the boundary between the two it keeps
    /// its own zone. `from == to` answers the tab's current zone. `None` when
    /// either index is off the strip. It never panics, even on a strip whose
    /// zones are out of order, which no mutator leaves.
    #[must_use]
    pub fn lands_parked(&self, from: usize, to: usize) -> Option<bool> {
        let was_parked = self.tabs.get(from)?.parked;
        self.tabs.get(to)?;
        let unparked_besides = self.first_parked().checked_sub(usize::from(!was_parked))?;
        Some(match to.cmp(&unparked_besides) {
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => was_parked,
            std::cmp::Ordering::Greater => true,
        })
    }

    /// Move the tab at `from` to index `to` of the resulting strip with
    /// `parked` as its state. The callers pick a `to` that keeps every parked
    /// tab after every unparked one. The active index follows its tab, unless
    /// that tab is now parked while unparked tabs remain: then the unparked tab
    /// in the slot it left takes over, else the last unparked one.
    fn relocate(&mut self, from: usize, to: usize, parked: bool) {
        let mut tab = self.tabs.remove(from);
        tab.parked = parked;
        self.tabs.insert(to, tab);
        self.active = shift_index(self.active, from, to);
        if self.tabs[self.active].parked && self.first_parked() > 0 {
            self.active = self.unparked_fallback(from);
        }
    }

    /// The unparked tab nearest `slot`: the one at it, else the last unparked
    /// one. The last tab when every tab is parked.
    fn unparked_fallback(&self, slot: usize) -> usize {
        let unparked = match self.first_parked() {
            0 => self.tabs.len(),
            count => count,
        };
        slot.min(unparked.saturating_sub(1))
    }

    /// Close the tab at `index` (FR5), returning the sessions it hosted so the
    /// caller can kill their PTYs. The active index is kept pointing at a valid
    /// tab: tabs after the removed one shift down, and closing the active tab
    /// focuses the one that slides into its slot (or the new last tab). An
    /// out-of-range index is a no-op returning an empty list.
    pub fn close_tab(&mut self, index: usize) -> Vec<SessionId> {
        if index >= self.tabs.len() {
            return Vec::new();
        }
        let sessions = self.tabs.remove(index).sessions();
        self.active = self.active_after_removing(index);
        sessions
    }

    /// The active index once the tab at `removed` has left the strip: the same
    /// tab when it survives, else the unparked tab nearest its slot.
    fn active_after_removing(&self, removed: usize) -> usize {
        index_after_removal(self.active, removed).unwrap_or_else(|| self.unparked_fallback(removed))
    }

    /// Session id of the focused pane in the active tab, if any.
    pub fn focused_session(&self) -> Option<SessionId> {
        self.tabs.get(self.active)?.focused_session()
    }

    /// Close the focused pane in the active tab (FR6), returning its session so
    /// the caller can kill the PTY. Its parent split collapses — the sibling
    /// subtree takes the split's place and gains focus. If the focused pane is
    /// the whole tab, the tab is closed (like [`Workspace::close_tab`]).
    pub fn close_focused(&mut self) -> Option<SessionId> {
        let path = self.tabs.get(self.active)?.focus.clone();
        self.close_leaf(self.active, path)
    }

    /// Close the leaf hosting `session`, wherever it lives — even an unfocused
    /// pane in an inactive tab (the auto-close after a clean shell exit). Same
    /// collapse rules as [`Workspace::close_focused`]. Returns the closed
    /// session, or `None` when no tab hosts it.
    pub fn close_pane_of(&mut self, session: SessionId) -> Option<SessionId> {
        let index = self.tab_of(session)?;
        let path = leaf_path_of(&self.tabs.get(index)?.root, session)?;
        self.close_leaf(index, path)
    }

    /// Close the leaf at `path` in the tab at `index`: its parent split
    /// collapses — the sibling subtree takes the split's place — or the whole
    /// tab closes (like [`Workspace::close_tab`]) when the leaf is the root.
    /// Focus follows the previously focused leaf through the collapse; when the
    /// closed leaf held it, the sibling subtree's first leaf gains it. Returns
    /// the removed session; `None` when `path` does not resolve to a leaf.
    fn close_leaf(&mut self, index: usize, mut path: Vec<Branch>) -> Option<SessionId> {
        // Read what we need first, releasing the borrow before any `self.*`.
        let (removed, focused) = {
            let tab = self.tabs.get(index)?;
            let removed = match navigate(&tab.root, &path)? {
                Pane::Leaf(session) => *session,
                Pane::Split { .. } => return None,
            };
            (removed, tab.focused_session())
        };
        if path.is_empty() {
            self.close_tab(index);
            return Some(removed);
        }
        let tab = self.tabs.get_mut(index)?;
        let branch = path.pop()?;
        let parent = navigate_mut(&mut tab.root, &path)?;
        // Replace the parent split with the sibling subtree (the other branch).
        let taken = std::mem::replace(parent, Pane::Leaf(removed));
        if let Pane::Split { a, b, .. } = taken {
            *parent = match branch {
                Branch::A => *b,
                Branch::B => *a,
            };
        }
        // A surviving focused leaf keeps focus through the collapse (its path
        // may have shifted); the closed leaf hands it to the sibling subtree,
        // descending to its first leaf.
        tab.focus = focused
            .filter(|f| *f != removed)
            .and_then(|f| leaf_path_of(&tab.root, f))
            .unwrap_or_else(|| {
                let mut focus = path;
                focus_first_leaf(&tab.root, &mut focus);
                focus
            });
        Some(removed)
    }

    /// Move focus to the next pane in the active tab (FR6), left to right and
    /// wrapping. No-op without an active tab.
    pub fn focus_next(&mut self) -> Option<()> {
        self.cycle_focus(Step::Next)
    }

    /// Move focus to the previous pane in the active tab (FR6), wrapping.
    pub fn focus_prev(&mut self) -> Option<()> {
        self.cycle_focus(Step::Prev)
    }

    /// Move focus to the leaf hosting `session` in the active tab
    /// (click-to-focus, FR6). Returns `None` — leaving focus untouched — when
    /// no active tab hosts it as a leaf.
    pub fn focus_pane_of(&mut self, session: SessionId) -> Option<()> {
        let tab = self.tabs.get_mut(self.active)?;
        let path = leaf_paths(&tab.root)
            .into_iter()
            .find(|p| matches!(navigate(&tab.root, p), Some(Pane::Leaf(s)) if *s == session))?;
        tab.focus = path;
        Some(())
    }

    /// Bring the pane hosting `session` into view wherever it lives — activating
    /// its tab first, then focusing the leaf. The counterpart of
    /// [`Workspace::focus_pane_of`], which only reaches the *active* tab because
    /// a click cannot land anywhere else; a caller addressing a pane by handle
    /// can. A parked tab is unparked. Returns `None` — leaving both the active
    /// tab and the focus untouched — when no tab hosts it.
    pub fn reveal_pane_of(&mut self, session: SessionId) -> Option<()> {
        let index = self.tab_of(session)?;
        self.activate(index)?;
        self.focus_pane_of(session)
    }

    /// Move pane focus one step in a spatial direction (FR6), cycling within
    /// the move's axis. It crosses the nearest ancestor split of that
    /// orientation into the sibling subtree, landing on the adjacent leaf; from
    /// the far edge it wraps to the opposite end. Returns `None` (focus
    /// unchanged) when no split of that orientation exists to move through.
    pub fn focus_dir(&mut self, dir: Direction) -> Option<()> {
        let (axis, toward) = dir.axis_toward();
        let tab = self.tabs.get_mut(self.active)?;
        tab.focus = step_across(&tab.root, &tab.focus, axis, toward)
            .or_else(|| wrap_around(&tab.root, &tab.focus, axis, toward))?;
        Some(())
    }

    fn cycle_focus(&mut self, step: Step) -> Option<()> {
        let tab = self.tabs.get_mut(self.active)?;
        let mut paths = leaf_paths(&tab.root);
        let current = paths.iter().position(|p| *p == tab.focus).unwrap_or(0);
        let next = step.index_after(current, paths.len())?;
        tab.focus = paths.swap_remove(next);
        Some(())
    }

    /// The active-tab index one `step` away among the unparked tabs, wrapping
    /// around both ends (FR9 `NextTab` / `PrevTab`). `None` when no tab is
    /// unparked, so the caller no-ops rather than switching to a tab that isn't
    /// in the cycle.
    #[must_use]
    pub fn cycled_tab(&self, step: Step) -> Option<usize> {
        step.index_after(self.active, self.first_parked())
    }
}

/// Where a tab index lands once the tab at `removed` is gone: unchanged
/// before it, one down after it, and `None` for the removed tab itself. The
/// one rule every index kept across a tab removal follows, in `core` and in
/// the shell's own tab-keyed state alike.
#[must_use]
pub fn index_after_removal(index: usize, removed: usize) -> Option<usize> {
    match index.cmp(&removed) {
        std::cmp::Ordering::Less => Some(index),
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(index - 1),
    }
}

/// Where an index `i` ends up after the tab at `from` is removed and
/// reinserted at `to` (see [`Workspace::move_tab`]). The moved tab lands on
/// `to`; every other index is mapped through the remove-then-insert shift.
fn shift_index(i: usize, from: usize, to: usize) -> usize {
    if i == from {
        return to;
    }
    // After removing `from`, indices past it slide down one…
    let after_remove = index_after_removal(i, from).unwrap_or(i);
    // …then inserting at `to` pushes indices at or beyond it up one.
    if after_remove >= to {
        after_remove + 1
    } else {
        after_remove
    }
}

/// Extend `focus` from the node it points at down to that subtree's first
/// (leftmost) leaf, so the focus path always ends on a leaf.
fn focus_first_leaf(root: &Pane, focus: &mut Vec<Branch>) {
    while let Some(Pane::Split { .. }) = navigate(root, focus) {
        focus.push(Branch::A);
    }
}

/// Path from `pane` down to the leaf holding `session`, if present.
fn leaf_path_of(pane: &Pane, session: SessionId) -> Option<Vec<Branch>> {
    match pane {
        Pane::Leaf(s) => (*s == session).then(Vec::new),
        Pane::Split { a, b, .. } => {
            [(Branch::A, a), (Branch::B, b)]
                .into_iter()
                .find_map(|(branch, side)| {
                    let mut path = leaf_path_of(side, session)?;
                    path.insert(0, branch);
                    Some(path)
                })
        }
    }
}

/// Whether the node at `path` is a split of the given orientation.
fn ancestor_is(root: &Pane, path: &[Branch], axis: SplitDir) -> bool {
    matches!(navigate(root, path), Some(Pane::Split { dir, .. }) if *dir == axis)
}

/// Descend from the node at `path` to the leaf on the edge nearest the pane we
/// arrived from, that is on the side opposite to `toward`: moving right lands
/// on the leftmost leaf, moving up on the bottommost.
fn descend_edge(root: &Pane, path: &mut Vec<Branch>, toward: Branch) {
    while let Some(Pane::Split { .. }) = navigate(root, path) {
        path.push(toward.opposite());
    }
}

/// A directional step: across the nearest ancestor split of `axis` where the
/// focused subtree sits on the branch we can leave toward `toward`.
fn step_across(
    root: &Pane,
    focus: &[Branch],
    axis: SplitDir,
    toward: Branch,
) -> Option<Vec<Branch>> {
    (0..focus.len()).rev().find_map(|i| {
        let parent = &focus[..i];
        (focus[i] == toward.opposite() && ancestor_is(root, parent, axis))
            .then(|| edge_leaf(root, parent, toward, toward))
    })
}

/// Already at the far edge: the opposite end of the outermost split of `axis`.
/// No such split → nothing to move through.
fn wrap_around(
    root: &Pane,
    focus: &[Branch],
    axis: SplitDir,
    toward: Branch,
) -> Option<Vec<Branch>> {
    (0..focus.len()).find_map(|i| {
        let parent = &focus[..i];
        ancestor_is(root, parent, axis).then(|| edge_leaf(root, parent, toward.opposite(), toward))
    })
}

/// The path into `branch` of the split at `parent`, descended to the edge leaf
/// a move `toward` arrives on.
fn edge_leaf(root: &Pane, parent: &[Branch], branch: Branch, toward: Branch) -> Vec<Branch> {
    let mut path = parent.to_vec();
    path.push(branch);
    descend_edge(root, &mut path, toward);
    path
}

/// Every leaf's path from the root, left to right.
fn leaf_paths(root: &Pane) -> Vec<Vec<Branch>> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    collect_paths(root, &mut path, &mut out);
    out
}

fn collect_paths(pane: &Pane, path: &mut Vec<Branch>, out: &mut Vec<Vec<Branch>>) {
    match pane {
        Pane::Leaf(_) => out.push(path.clone()),
        Pane::Split { a, b, .. } => {
            path.push(Branch::A);
            collect_paths(a, path, out);
            path.pop();
            path.push(Branch::B);
            collect_paths(b, path, out);
            path.pop();
        }
    }
}

/// A split ratio as the workspace keeps it: neither pane under ten percent.
fn kept_ratio(ratio: f32) -> f32 {
    ratio.clamp(0.1, 0.9)
}

/// `pane` with every ratio kept in range, an unreadable one split evenly.
fn with_kept_ratios(pane: Pane) -> Pane {
    match pane {
        Pane::Leaf(_) => pane,
        Pane::Split { dir, ratio, a, b } => Pane::Split {
            dir,
            ratio: if ratio.is_finite() {
                kept_ratio(ratio)
            } else {
                0.5
            },
            a: Box::new(with_kept_ratios(*a)),
            b: Box::new(with_kept_ratios(*b)),
        },
    }
}

/// The session of the first (leftmost, topmost) leaf under `pane`.
fn first_leaf(mut pane: &Pane) -> SessionId {
    loop {
        match pane {
            Pane::Leaf(session) => return *session,
            Pane::Split { a, .. } => pane = a,
        }
    }
}

/// The path from `pane` down to its first leaf.
fn first_leaf_path(mut pane: &Pane) -> Vec<Branch> {
    let mut path = Vec::new();
    while let Pane::Split { a, .. } = pane {
        path.push(Branch::A);
        pane = a;
    }
    path
}

fn navigate<'a>(mut pane: &'a Pane, path: &[Branch]) -> Option<&'a Pane> {
    for step in path {
        pane = match (pane, step) {
            (Pane::Split { a, .. }, Branch::A) => a.as_ref(),
            (Pane::Split { b, .. }, Branch::B) => b.as_ref(),
            _ => return None,
        };
    }
    Some(pane)
}

fn navigate_mut<'a>(mut pane: &'a mut Pane, path: &[Branch]) -> Option<&'a mut Pane> {
    for step in path {
        pane = match (pane, step) {
            (Pane::Split { a, .. }, Branch::A) => a.as_mut(),
            (Pane::Split { b, .. }, Branch::B) => b.as_mut(),
            _ => return None,
        };
    }
    Some(pane)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(n: u64) -> SessionId {
        SessionId(NonZeroU64::new(n).unwrap_or(NonZeroU64::MIN))
    }

    /// A workspace holding `count` single-session tabs, active on the last one.
    fn workspace_with_tabs(count: usize) -> Workspace {
        let mut ws = Workspace::new();
        for n in 0..count {
            ws.open(sid(n as u64 + 1), format!("t{n}"));
        }
        ws
    }

    /// One tab, `1 | 2`, focus on 2.
    fn side_by_side() -> Workspace {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.split(SplitDir::Vertical, sid(2));
        ws
    }

    /// One tab, `1 | 2 | 3` built as `Vertical(1, Vertical(2, 3))`, focus on 3.
    fn three_columns() -> Workspace {
        let mut ws = side_by_side();
        ws.split(SplitDir::Vertical, sid(3));
        ws
    }

    /// One tab, 1 on the left, 2 stacked over 3 on the right:
    /// `Vertical(1, Horizontal(2, 3))`, focus on 3.
    fn column_beside_stack() -> Workspace {
        let mut ws = side_by_side();
        ws.split(SplitDir::Horizontal, sid(3));
        ws
    }

    /// One tab, 1 on top, `2 | 3` below: `Horizontal(1, Vertical(2, 3))`,
    /// focus on 3.
    fn row_over_columns() -> Workspace {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.split(SplitDir::Horizontal, sid(2));
        ws.split(SplitDir::Vertical, sid(3));
        ws
    }

    /// Move focus one step and report where it landed; `None` when the move
    /// was refused.
    fn step(ws: &mut Workspace, dir: Direction) -> Option<SessionId> {
        ws.focus_dir(dir)?;
        ws.focused_session()
    }

    /// The ratio of the split at `path` in the first tab.
    fn ratio_at(ws: &Workspace, path: &[Branch]) -> f32 {
        match navigate(&ws.tabs[0].root, path) {
            Some(Pane::Split { ratio, .. }) => *ratio,
            other => panic!("expected a split at {path:?}, found {other:?}"),
        }
    }

    /// The session ids hosted by each tab, in tab order — a stable identity to
    /// assert reordering against.
    fn tab_order(ws: &Workspace) -> Vec<SessionId> {
        ws.tabs.iter().map(|t| t.sessions()[0]).collect()
    }

    #[test]
    fn open_creates_a_tab_with_the_session() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.active, 0);
        assert_eq!(ws.focused_session(), Some(sid(1)));
        assert!(ws.tabs[0].focus.is_empty());
    }

    #[test]
    fn cycled_tab_wraps_both_directions() {
        assert_eq!(
            Workspace::new().cycled_tab(Step::Next),
            None,
            "no tabs: nothing to cycle to"
        );
        assert_eq!(Workspace::new().cycled_tab(Step::Prev), None);

        let ws = workspace_with_tabs(3);
        assert_eq!(ws.active, 2);

        // Forward from the last tab wraps round to the first.
        assert_eq!(ws.cycled_tab(Step::Next), Some(0));
        // Backward from the last tab steps to the middle.
        assert_eq!(ws.cycled_tab(Step::Prev), Some(1));
        // The query is pure — it computes the next index, it does not switch.
        assert_eq!(ws.active, 2, "cycled_tab must not mutate the active index");
    }

    #[test]
    fn a_live_title_relabels_the_hosting_tab_only() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "first");
        ws.open(sid(2), "second");
        // A split: tab 1 now hosts sid(2) and sid(3).
        ws.split(SplitDir::Vertical, sid(3));

        assert_eq!(ws.set_live_title(sid(1), "renamed"), Some(()));
        assert_eq!(ws.tabs[0].title, "renamed");
        // A tab is named after its first pane: a pane split beside it — a
        // shell whose prompt sets a title — does not speak for the tab.
        assert_eq!(ws.set_live_title(sid(3), "zsh"), Some(()));
        assert_eq!(ws.tabs[1].title, "second");
        assert_eq!(ws.set_live_title(sid(2), "split title"), Some(()));
        assert_eq!(ws.tabs[1].title, "split title");
        // The untouched tab keeps its title.
        assert_eq!(ws.tabs[0].title, "renamed");
        // An unknown session changes nothing.
        assert_eq!(ws.set_live_title(sid(99), "ghost"), None);
        assert_eq!(ws.tabs[0].title, "renamed");
        assert_eq!(ws.tabs[1].title, "split title");
    }

    #[test]
    fn focus_dir_moves_and_wraps_within_a_vertical_split() {
        let mut ws = side_by_side();
        assert_eq!(ws.focused_session(), Some(sid(2)));

        // Left steps to the pane on the left.
        assert_eq!(step(&mut ws, Direction::Left), Some(sid(1)));
        // Left again from the leftmost wraps to the rightmost (cyclic on axis).
        assert_eq!(step(&mut ws, Direction::Left), Some(sid(2)));
        // Right from the rightmost wraps back to the leftmost.
        assert_eq!(step(&mut ws, Direction::Right), Some(sid(1)));

        // Up/Down find no horizontal split to traverse: no-op.
        assert_eq!(step(&mut ws, Direction::Up), None);
        assert_eq!(ws.focused_session(), Some(sid(1)));
    }

    #[test]
    fn focus_dir_traverses_the_matching_axis_in_a_nested_split() {
        let mut ws = column_beside_stack();
        assert_eq!(ws.focused_session(), Some(sid(3)));

        // Up moves within the right column to sid2.
        assert_eq!(step(&mut ws, Direction::Up), Some(sid(2)));
        // Left crosses the outer vertical split to the left column.
        assert_eq!(step(&mut ws, Direction::Left), Some(sid(1)));
        // Right crosses back; the edge nearest the divider is sid2 (top-right).
        assert_eq!(step(&mut ws, Direction::Right), Some(sid(2)));
    }

    #[test]
    fn focus_dir_wraps_through_the_outermost_split_of_its_axis() {
        let mut ws = three_columns();
        assert_eq!(ws.focused_session(), Some(sid(3)));

        // Right from the rightmost wraps to the far left, not to the inner
        // split's own left pane (sid2).
        assert_eq!(step(&mut ws, Direction::Right), Some(sid(1)));
        // Left from the leftmost wraps to the far right, descending through the
        // inner split to its rightmost leaf.
        assert_eq!(step(&mut ws, Direction::Left), Some(sid(3)));
    }

    #[test]
    fn focus_pane_of_moves_focus_to_the_leaf_hosting_a_session() {
        let mut ws = side_by_side();
        assert_eq!(ws.focused_session(), Some(sid(2)));

        // Clicking the other pane jumps focus to the leaf hosting sid(1).
        assert_eq!(ws.focus_pane_of(sid(1)), Some(()));
        assert_eq!(ws.focused_session(), Some(sid(1)));

        // A session absent from the active tab is a no-op, focus unchanged.
        assert_eq!(ws.focus_pane_of(sid(99)), None);
        assert_eq!(ws.focused_session(), Some(sid(1)));
    }

    #[test]
    fn focus_pane_of_does_not_reach_into_another_tab() {
        let mut ws = workspace_with_tabs(2);
        // A click cannot land on an inactive tab, so this stays a no-op — the
        // guarantee `reveal_pane_of` exists to lift.
        assert_eq!(ws.focus_pane_of(sid(1)), None);
        assert_eq!(ws.active, 1);
        assert_eq!(ws.focused_session(), Some(sid(2)));
    }

    #[test]
    fn reveal_pane_of_activates_the_tab_hosting_the_session() {
        let mut ws = side_by_side();
        ws.open(sid(3), "b");
        assert_eq!(ws.active, 1);

        // Reaching back into the first tab activates it *and* lands focus on the
        // named leaf — not merely on whatever that tab last focused.
        assert_eq!(ws.reveal_pane_of(sid(1)), Some(()));
        assert_eq!(ws.active, 0);
        assert_eq!(ws.focused_session(), Some(sid(1)));
    }

    #[test]
    fn reveal_pane_of_leaves_everything_untouched_for_an_unhosted_session() {
        let mut ws = workspace_with_tabs(2);
        assert_eq!(ws.reveal_pane_of(sid(99)), None);
        assert_eq!(ws.active, 1, "the active tab must not move");
        assert_eq!(ws.focused_session(), Some(sid(2)));
    }

    #[test]
    fn rename_tab_overrides_the_derived_title_while_keeping_it() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "derived");
        assert_eq!(ws.rename_tab(0, "custom"), Some(()));
        assert_eq!(ws.tabs[0].display_title(), "custom");
        // the derived title is retained underneath the override.
        assert_eq!(ws.tabs[0].title, "derived");
    }

    #[test]
    fn a_custom_title_wins_over_a_later_derived_update() {
        // The manual name must not be clobbered by a subsequent OSC/digest
        // title — the user override wins.
        let mut ws = Workspace::new();
        ws.open(sid(1), "derived");
        ws.rename_tab(0, "custom");
        assert_eq!(ws.set_live_title(sid(1), "new derived"), Some(()));
        assert_eq!(ws.tabs[0].display_title(), "custom");
        assert_eq!(ws.tabs[0].title, "new derived");
    }

    #[test]
    fn a_blank_rename_reverts_to_the_derived_title() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "derived");
        ws.rename_tab(0, "custom");
        // whitespace-only clears the override.
        assert_eq!(ws.rename_tab(0, "   "), Some(()));
        assert_eq!(ws.tabs[0].custom_title, None);
        assert_eq!(ws.tabs[0].display_title(), "derived");
    }

    #[test]
    fn renaming_to_the_derived_title_stores_no_override() {
        // An override equal to the derived title is redundant: storing it would
        // freeze the title against later relabels. Even with stray whitespace,
        // the tab must keep tracking its derived title.
        let mut ws = Workspace::new();
        ws.open(sid(1), "derived");
        assert_eq!(ws.rename_tab(0, "  derived  "), Some(()));
        assert_eq!(ws.tabs[0].custom_title, None);
        assert_eq!(ws.tabs[0].display_title(), "derived");
    }

    #[test]
    fn tab_of_locates_the_hosting_tab_as_a_stable_rename_anchor() {
        let ws = workspace_with_tabs(2);
        assert_eq!(ws.tab_of(sid(2)), Some(1));
        assert_eq!(ws.tab_of(sid(9)), None);
    }

    #[test]
    fn session_title_reflects_a_custom_tab_name() {
        // The notification title reads through session_title; a renamed tab must
        // announce its custom name, not the derived one it hides.
        let mut ws = Workspace::new();
        ws.open(sid(1), "derived");
        ws.rename_tab(0, "custom");
        assert_eq!(ws.session_title(sid(1)), Some("custom"));
    }

    #[test]
    fn rename_tab_trims_and_ignores_an_out_of_range_index() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "derived");
        assert_eq!(ws.rename_tab(0, "  spaced  "), Some(()));
        assert_eq!(ws.tabs[0].display_title(), "spaced");
        assert_eq!(ws.rename_tab(9, "x"), None);
        assert_eq!(ws.tabs[0].display_title(), "spaced");
    }

    #[test]
    fn open_multiple_makes_latest_active() {
        let ws = workspace_with_tabs(3);
        assert_eq!(ws.tabs.len(), 3);
        assert_eq!(ws.active, 2);
        assert_eq!(ws.focused_session(), Some(sid(3)));
    }

    #[test]
    fn split_wraps_leaf_into_split_and_focuses_b() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        assert!(ws.split(SplitDir::Vertical, sid(2)).is_some());
        let tab = &ws.tabs[0];
        match &tab.root {
            Pane::Split { dir, a, b, .. } => {
                assert_eq!(*dir, SplitDir::Vertical);
                assert_eq!(**a, Pane::Leaf(sid(1)));
                assert_eq!(**b, Pane::Leaf(sid(2)));
            }
            Pane::Leaf(_) => panic!("expected a split at the root"),
        }
        assert_eq!(tab.focus, vec![Branch::B]);
        assert_eq!(ws.focused_session(), Some(sid(2)));
    }

    #[test]
    fn set_split_ratio_updates_the_requested_tab_and_nested_path() {
        let mut ws = column_beside_stack();
        ws.open(sid(4), "b");

        assert_eq!(ws.set_split_ratio(0, &[Branch::B], 0.25), Some(()));
        assert_eq!(ratio_at(&ws, &[]), 0.5, "the root split remains unchanged");
        assert_eq!(ratio_at(&ws, &[Branch::B]), 0.25);
        assert_eq!(ws.active, 1, "the explicitly targeted tab stays in place");
    }

    #[test]
    fn set_split_ratio_keeps_each_pane_at_least_ten_percent() {
        let mut ws = side_by_side();
        assert_eq!(ws.set_split_ratio(0, &[], 0.0), Some(()));
        assert_eq!(ratio_at(&ws, &[]), 0.1);
        assert_eq!(ws.set_split_ratio(0, &[], 1.0), Some(()));
        assert_eq!(ratio_at(&ws, &[]), 0.9);
    }

    #[test]
    fn set_split_ratio_rejects_non_finite_values() {
        let mut ws = side_by_side();
        assert_eq!(ws.set_split_ratio(0, &[], f32::NAN), None);
        assert_eq!(ratio_at(&ws, &[]), 0.5);
    }

    #[test]
    fn set_split_ratio_rejects_a_leaf_or_missing_tab() {
        let mut ws = side_by_side();
        assert_eq!(ws.set_split_ratio(0, &[Branch::A], 0.25), None);
        assert_eq!(ws.set_split_ratio(9, &[], 0.25), None);
        assert_eq!(ratio_at(&ws, &[]), 0.5);
    }

    #[test]
    fn nested_splits_keep_focus_on_newest_leaf() {
        let ws = row_over_columns();
        // focus is at B,B inside two nested splits
        assert_eq!(ws.tabs[0].focus, vec![Branch::B, Branch::B]);
        assert_eq!(ws.focused_session(), Some(sid(3)));
    }

    #[test]
    fn split_on_empty_workspace_is_a_noop() {
        let mut ws = Workspace::new();
        // No active tab; split must fail cleanly (no panic, no state change).
        assert!(ws.split(SplitDir::Vertical, sid(1)).is_none());
        assert!(ws.tabs.is_empty());
    }

    #[test]
    fn activate_switches_the_active_tab_and_rejects_out_of_range() {
        let mut ws = workspace_with_tabs(2);
        assert_eq!(ws.active, 1);
        assert!(ws.activate(0).is_some());
        assert_eq!(ws.active, 0);
        assert_eq!(ws.focused_session(), Some(sid(1)));
        // Out of range leaves the active tab untouched.
        assert!(ws.activate(9).is_none());
        assert_eq!(ws.active, 0);
    }

    proptest::proptest! {
        #[test]
        fn an_index_keeps_naming_its_tab_across_a_removal(
            len in 1usize..12,
            seed in 0usize..1000,
            removed_seed in 0usize..1000,
        ) {
            let tabs: Vec<usize> = (0..len).collect();
            let index = seed % len;
            let removed = removed_seed % len;
            let mut after = tabs.clone();
            after.remove(removed);
            match index_after_removal(index, removed) {
                Some(shifted) => proptest::prop_assert_eq!(after[shifted], tabs[index]),
                None => proptest::prop_assert_eq!(index, removed),
            }
        }
    }

    proptest::proptest! {
        /// Regression guard for the method the number-row jump leans on:
        /// `activate` selects an in-range index, rejects anything
        /// beyond the last tab without moving, and never panics.
        #[test]
        fn activate_selects_within_range_and_rejects_beyond(
            count in 1usize..12,
            index in 0usize..32,
        ) {
            let mut ws = workspace_with_tabs(count);
            let before = ws.active;
            let result = ws.activate(index);
            if index < count {
                proptest::prop_assert_eq!(result, Some(()));
                proptest::prop_assert_eq!(ws.active, index);
            } else {
                proptest::prop_assert_eq!(result, None);
                proptest::prop_assert_eq!(ws.active, before);
            }
        }

        /// Activating the same tab twice is idempotent.
        #[test]
        fn activate_is_idempotent(count in 1usize..12, index in 0usize..12) {
            let mut ws = workspace_with_tabs(count);
            let first = ws.activate(index);
            let active_after_first = ws.active;
            let second = ws.activate(index);
            proptest::prop_assert_eq!(first, second);
            proptest::prop_assert_eq!(ws.active, active_after_first);
        }
    }

    #[test]
    fn tab_sessions_lists_every_leaf_in_pane_order() {
        let ws = column_beside_stack();
        assert_eq!(ws.tabs[0].sessions(), vec![sid(1), sid(2), sid(3)]);
    }

    #[test]
    fn close_tab_returns_its_sessions_and_keeps_active_valid() {
        let mut ws = workspace_with_tabs(3);
        // Active is the last tab. Closing an earlier tab shifts it down.
        assert_eq!(ws.active, 2);
        assert_eq!(ws.close_tab(0), vec![sid(1)]);
        assert_eq!(ws.tabs.len(), 2);
        assert_eq!(ws.active, 1);
        assert_eq!(ws.focused_session(), Some(sid(3)));
    }

    #[test]
    fn closing_the_active_last_tab_focuses_the_new_last() {
        let mut ws = workspace_with_tabs(2);
        assert_eq!(ws.active, 1);
        assert_eq!(ws.close_tab(1), vec![sid(2)]);
        assert_eq!(ws.active, 0);
        assert_eq!(ws.focused_session(), Some(sid(1)));
    }

    #[test]
    fn close_tab_kills_all_sessions_in_a_split() {
        let mut ws = side_by_side();
        assert_eq!(ws.close_tab(0), vec![sid(1), sid(2)]);
        assert!(ws.tabs.is_empty());
        assert_eq!(ws.active, 0);
    }

    #[test]
    fn tab_of_locates_the_hosting_tab_including_inside_a_split() {
        let mut ws = workspace_with_tabs(2);
        ws.split(SplitDir::Vertical, sid(3)); // sid 3 joins tab 1 as a split
        assert_eq!(ws.tab_of(sid(1)), Some(0));
        assert_eq!(ws.tab_of(sid(2)), Some(1));
        assert_eq!(ws.tab_of(sid(3)), Some(1), "a split member maps to its tab");
        assert_eq!(ws.tab_of(sid(9)), None, "an unopened session has no tab");
    }

    #[test]
    fn close_tab_out_of_range_is_a_noop() {
        let mut ws = workspace_with_tabs(1);
        assert!(ws.close_tab(5).is_empty());
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.active, 0);
    }

    #[test]
    fn close_focused_collapses_the_split_onto_the_sibling() {
        let mut ws = side_by_side();
        // Closing the focused pane removes sid 2 and leaves just sid 1.
        assert_eq!(ws.close_focused(), Some(sid(2)));
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.tabs[0].root, Pane::Leaf(sid(1)));
        assert_eq!(ws.focused_session(), Some(sid(1)));
    }

    #[test]
    fn close_focused_on_a_single_pane_tab_closes_the_tab() {
        let mut ws = workspace_with_tabs(2);
        // Active tab is the single-leaf tab 1; closing its pane closes the tab.
        assert_eq!(ws.close_focused(), Some(sid(2)));
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.focused_session(), Some(sid(1)));
    }

    #[test]
    fn close_focused_in_a_nested_split_keeps_a_leaf_focused() {
        let mut ws = row_over_columns();
        assert_eq!(ws.close_focused(), Some(sid(3)));
        // The sibling (sid 2) takes the inner split's place and is focused.
        assert_eq!(ws.focused_session(), Some(sid(2)));
        assert_eq!(ws.tabs[0].sessions(), vec![sid(1), sid(2)]);
    }

    #[test]
    fn close_pane_of_collapses_an_unfocused_leaf_and_keeps_focus() {
        let mut ws = side_by_side();
        // Closing the *unfocused* pane (sid 1) must not steal focus from sid 2.
        assert_eq!(ws.close_pane_of(sid(1)), Some(sid(1)));
        assert_eq!(ws.tabs[0].root, Pane::Leaf(sid(2)));
        assert_eq!(ws.focused_session(), Some(sid(2)));
    }

    #[test]
    fn close_pane_of_the_focused_leaf_moves_focus_to_the_sibling() {
        let mut ws = side_by_side();
        assert_eq!(ws.close_pane_of(sid(2)), Some(sid(2)));
        assert_eq!(ws.tabs[0].root, Pane::Leaf(sid(1)));
        assert_eq!(ws.focused_session(), Some(sid(1)));
    }

    #[test]
    fn close_pane_of_reaches_into_an_inactive_tab_without_switching() {
        let mut ws = side_by_side();
        ws.open(sid(3), "b"); // active tab is now "b"
        assert_eq!(ws.close_pane_of(sid(2)), Some(sid(2)));
        assert_eq!(ws.tabs[0].root, Pane::Leaf(sid(1)));
        // The active tab and its focus are untouched.
        assert_eq!(ws.active, 1);
        assert_eq!(ws.focused_session(), Some(sid(3)));
    }

    #[test]
    fn close_pane_of_a_root_leaf_closes_the_whole_tab() {
        let mut ws = workspace_with_tabs(2);
        assert_eq!(ws.close_pane_of(sid(1)), Some(sid(1)));
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.focused_session(), Some(sid(2)));
    }

    #[test]
    fn close_pane_of_an_unknown_session_is_a_noop() {
        let mut ws = workspace_with_tabs(1);
        assert_eq!(ws.close_pane_of(sid(9)), None);
        assert_eq!(ws.tabs.len(), 1);
    }

    #[test]
    fn focus_next_and_prev_cycle_through_panes() {
        let mut ws = three_columns();
        assert_eq!(ws.focused_session(), Some(sid(3)));
        // Wrap forward to the first leaf, then walk back.
        assert!(ws.focus_next().is_some());
        assert_eq!(ws.focused_session(), Some(sid(1)));
        assert!(ws.focus_prev().is_some());
        assert_eq!(ws.focused_session(), Some(sid(3)));
        assert!(ws.focus_prev().is_some());
        assert_eq!(ws.focused_session(), Some(sid(2)));
    }

    #[test]
    fn focus_moves_are_noops_without_an_active_tab() {
        let mut ws = Workspace::new();
        assert!(ws.focus_next().is_none());
        assert!(ws.close_focused().is_none());
    }

    #[test]
    fn move_tab_shifts_a_tab_to_the_right() {
        let mut ws = workspace_with_tabs(4); // [1,2,3,4], active = 3
        assert_eq!(ws.move_tab(1, 2), Some(()));
        assert_eq!(tab_order(&ws), vec![sid(1), sid(3), sid(2), sid(4)]);
        // The active tab (sid 4) only shifted, so `active` still points at it.
        assert_eq!(ws.focused_session(), Some(sid(4)));
    }

    #[test]
    fn move_tab_shifts_a_tab_to_the_left() {
        let mut ws = workspace_with_tabs(4);
        assert_eq!(ws.move_tab(2, 0), Some(()));
        assert_eq!(tab_order(&ws), vec![sid(3), sid(1), sid(2), sid(4)]);
    }

    #[test]
    fn move_tab_follows_the_moved_active_tab() {
        let mut ws = workspace_with_tabs(3); // active = 2 (sid 3)
        assert_eq!(ws.move_tab(2, 0), Some(()));
        assert_eq!(tab_order(&ws), vec![sid(3), sid(1), sid(2)]);
        // Dragging the active tab keeps it active at its new slot.
        assert_eq!(ws.active, 0);
        assert_eq!(ws.focused_session(), Some(sid(3)));
    }

    #[test]
    fn move_tab_tracks_active_when_a_tab_jumps_over_it() {
        let mut ws = workspace_with_tabs(3);
        assert!(ws.activate(1).is_some()); // active = sid 2
        // Move sid 1 to the end: [2,3,1]; active must still be sid 2.
        assert_eq!(ws.move_tab(0, 2), Some(()));
        assert_eq!(tab_order(&ws), vec![sid(2), sid(3), sid(1)]);
        assert_eq!(ws.focused_session(), Some(sid(2)));
    }

    #[test]
    fn move_tab_same_index_is_a_noop() {
        let mut ws = workspace_with_tabs(3);
        assert_eq!(ws.move_tab(1, 1), Some(()));
        assert_eq!(tab_order(&ws), vec![sid(1), sid(2), sid(3)]);
        assert_eq!(ws.active, 2);
    }

    #[test]
    fn move_tab_out_of_range_changes_nothing() {
        let mut ws = workspace_with_tabs(2);
        assert!(ws.move_tab(0, 5).is_none());
        assert!(ws.move_tab(5, 0).is_none());
        assert_eq!(tab_order(&ws), vec![sid(1), sid(2)]);
        assert_eq!(ws.active, 1);
    }

    proptest::proptest! {
        /// Reordering preserves the multiset of tabs and always keeps the same
        /// tab active — never panics, never loses or duplicates a tab.
        #[test]
        fn move_tab_is_a_permutation_that_preserves_the_active_tab(
            count in 1usize..8,
            from in 0usize..8,
            to in 0usize..8,
        ) {
            let mut ws = workspace_with_tabs(count);
            let active_before = ws.tabs[ws.active].sessions()[0];
            let mut before = tab_order(&ws);
            let result = ws.move_tab(from, to);
            if from < count && to < count {
                proptest::prop_assert_eq!(result, Some(()));
                // Same set of tabs, and the active tab is unchanged in identity.
                let mut after = tab_order(&ws);
                before.sort_by_key(|s| s.0.get());
                after.sort_by_key(|s| s.0.get());
                proptest::prop_assert_eq!(before, after);
                proptest::prop_assert_eq!(ws.tabs[ws.active].sessions()[0], active_before);
            } else {
                proptest::prop_assert_eq!(result, None);
            }
        }
    }

    mod park {
        use proptest::collection::vec;
        use proptest::prelude::*;

        use super::*;

        /// `unparked` then `parked` single-session tabs, ids 1.. in strip
        /// order, active on the last unparked tab, or on the last tab when
        /// every one is parked.
        fn strip(unparked: usize, parked: usize) -> Workspace {
            let mut ws = workspace_with_tabs(unparked + parked);
            for tab in &mut ws.tabs[unparked..] {
                tab.parked = true;
            }
            ws.active = unparked
                .checked_sub(1)
                .unwrap_or(ws.tabs.len().saturating_sub(1));
            ws
        }

        fn parked_ids(ws: &Workspace) -> Vec<SessionId> {
            ws.tabs
                .iter()
                .filter(|tab| tab.is_parked())
                .map(Tab::first_session)
                .collect()
        }

        fn active_id(ws: &Workspace) -> SessionId {
            ws.tabs[ws.active].first_session()
        }

        /// The ordering rules every mutator keeps: parked tabs form the strip's
        /// tail, `active` names a tab, and that tab is parked only when every
        /// tab is.
        fn strip_rules_hold(ws: &Workspace) -> Result<(), String> {
            let boundary = ws.first_parked();
            if ws.tabs[boundary..].iter().any(|tab| !tab.is_parked()) {
                return Err(format!("an unparked tab sits after index {boundary}"));
            }
            if ws.tabs.is_empty() {
                return Ok(());
            }
            if ws.active >= ws.tabs.len() {
                return Err(format!("active {} is out of range", ws.active));
            }
            if ws.tabs[ws.active].is_parked() && boundary > 0 {
                return Err("the active tab is parked while unparked tabs remain".into());
            }
            Ok(())
        }

        #[test]
        fn parking_moves_a_tab_to_the_front_of_the_parked_group() {
            let mut ws = strip(3, 1);
            assert_eq!(ws.park(0), Some(()));
            assert_eq!(tab_order(&ws), vec![sid(2), sid(3), sid(1), sid(4)]);
            assert_eq!(parked_ids(&ws), vec![sid(1), sid(4)]);
            assert_eq!(ws.first_parked(), 2);
            assert_eq!(
                active_id(&ws),
                sid(3),
                "parking another tab keeps the active one"
            );
        }

        #[test]
        fn parking_the_active_tab_activates_the_unparked_tab_sliding_into_its_slot() {
            let mut ws = strip(3, 0);
            ws.activate(1);
            assert_eq!(ws.park(1), Some(()));
            assert_eq!(tab_order(&ws), vec![sid(1), sid(3), sid(2)]);
            assert_eq!(active_id(&ws), sid(3));
        }

        #[test]
        fn parking_the_last_unparked_active_tab_activates_the_new_last_unparked() {
            let mut ws = strip(3, 1);
            assert_eq!(ws.park(2), Some(()));
            assert_eq!(parked_ids(&ws), vec![sid(3), sid(4)]);
            assert_eq!(active_id(&ws), sid(2));
        }

        #[test]
        fn parking_the_only_unparked_tab_keeps_it_active() {
            let mut ws = strip(1, 1);
            assert_eq!(ws.park(0), Some(()));
            assert_eq!(parked_ids(&ws), vec![sid(1), sid(2)]);
            assert_eq!(ws.first_parked(), 0);
            assert_eq!(active_id(&ws), sid(1));
        }

        #[test]
        fn park_refuses_a_parked_or_missing_tab() {
            let mut ws = strip(2, 1);
            assert_eq!(ws.park(2), None);
            assert_eq!(ws.park(9), None);
            assert_eq!(tab_order(&ws), vec![sid(1), sid(2), sid(3)]);
            assert_eq!(parked_ids(&ws), vec![sid(3)]);
            assert_eq!(active_id(&ws), sid(2));
        }

        #[test]
        fn unparking_moves_a_tab_to_the_end_of_the_unparked_group() {
            let mut ws = strip(2, 2);
            assert_eq!(ws.unpark(3), Some(2));
            assert_eq!(tab_order(&ws), vec![sid(1), sid(2), sid(4), sid(3)]);
            assert_eq!(parked_ids(&ws), vec![sid(3)]);
            assert_eq!(active_id(&ws), sid(2), "unparking does not activate");
        }

        #[test]
        fn unpark_refuses_an_unparked_or_missing_tab() {
            let mut ws = strip(2, 1);
            assert_eq!(ws.unpark(0), None);
            assert_eq!(ws.unpark(9), None);
            assert_eq!(parked_ids(&ws), vec![sid(3)]);
        }

        #[test]
        fn activating_a_parked_tab_unparks_it() {
            let mut ws = strip(2, 2);
            assert_eq!(ws.activate(3), Some(()));
            assert_eq!(tab_order(&ws), vec![sid(1), sid(2), sid(4), sid(3)]);
            assert_eq!(parked_ids(&ws), vec![sid(3)]);
            assert_eq!(active_id(&ws), sid(4));
        }

        #[test]
        fn new_tabs_open_before_the_parked_group() {
            let mut ws = strip(2, 1);
            ws.open(sid(9), "nine");
            assert_eq!(tab_order(&ws), vec![sid(1), sid(2), sid(9), sid(3)]);
            assert_eq!(active_id(&ws), sid(9));
            assert_eq!(ws.append(sid(8), "eight"), 3);
            assert_eq!(tab_order(&ws), vec![sid(1), sid(2), sid(9), sid(8), sid(3)]);
            assert_eq!(active_id(&ws), sid(9));
            assert_eq!(parked_ids(&ws), vec![sid(3)]);
        }

        #[test]
        fn closing_the_active_tab_never_falls_back_onto_a_parked_tab() {
            let mut ws = strip(2, 1);
            assert_eq!(ws.close_tab(1), vec![sid(2)]);
            assert_eq!(tab_order(&ws), vec![sid(1), sid(3)]);
            assert_eq!(active_id(&ws), sid(1));
        }

        #[test]
        fn the_tab_cycle_skips_parked_tabs() {
            let ws = strip(3, 2);
            assert_eq!(ws.cycled_tab(Step::Next), Some(0));
            assert_eq!(ws.cycled_tab(Step::Prev), Some(1));
        }

        #[test]
        fn the_tab_cycle_has_nowhere_to_go_when_every_tab_is_parked() {
            let ws = strip(0, 2);
            assert_eq!(ws.cycled_tab(Step::Next), None);
            assert_eq!(ws.cycled_tab(Step::Prev), None);
        }

        #[test]
        fn revealing_a_pane_in_a_parked_tab_unparks_it() {
            let mut ws = strip(2, 1);
            assert_eq!(ws.reveal_pane_of(sid(3)), Some(()));
            assert_eq!(parked_ids(&ws), vec![]);
            assert_eq!(active_id(&ws), sid(3));
        }

        #[test]
        fn dragging_a_tab_into_the_parked_zone_parks_it() {
            let mut ws = strip(3, 2);
            assert_eq!(ws.move_tab(0, 3), Some(()));
            assert_eq!(tab_order(&ws), vec![sid(2), sid(3), sid(4), sid(1), sid(5)]);
            assert_eq!(parked_ids(&ws), vec![sid(4), sid(1), sid(5)]);
            assert_eq!(active_id(&ws), sid(3));
        }

        #[test]
        fn dragging_a_parked_tab_into_the_unparked_zone_unparks_it() {
            let mut ws = strip(3, 2);
            assert_eq!(ws.move_tab(4, 0), Some(()));
            assert_eq!(tab_order(&ws), vec![sid(5), sid(1), sid(2), sid(3), sid(4)]);
            assert_eq!(parked_ids(&ws), vec![sid(4)]);
            assert_eq!(active_id(&ws), sid(3));
        }

        #[test]
        fn a_drop_on_the_boundary_keeps_the_tab_in_its_zone() {
            let mut ws = strip(3, 2);
            assert_eq!(ws.move_tab(0, 2), Some(()));
            assert_eq!(tab_order(&ws), vec![sid(2), sid(3), sid(1), sid(4), sid(5)]);
            assert_eq!(parked_ids(&ws), vec![sid(4), sid(5)]);

            let mut ws = strip(3, 2);
            assert_eq!(ws.move_tab(4, 3), Some(()));
            assert_eq!(tab_order(&ws), vec![sid(1), sid(2), sid(3), sid(5), sid(4)]);
            assert_eq!(parked_ids(&ws), vec![sid(5), sid(4)]);
        }

        #[test]
        fn dragging_the_active_tab_into_the_parked_zone_activates_the_last_unparked() {
            let mut ws = strip(3, 2);
            assert_eq!(ws.move_tab(2, 4), Some(()));
            assert_eq!(tab_order(&ws), vec![sid(1), sid(2), sid(4), sid(5), sid(3)]);
            assert_eq!(parked_ids(&ws), vec![sid(4), sid(5), sid(3)]);
            assert_eq!(active_id(&ws), sid(2));
        }

        #[test]
        fn dragging_among_parked_tabs_keeps_the_active_one_when_every_tab_is_parked() {
            let mut ws = strip(0, 3);
            assert_eq!(active_id(&ws), sid(3));
            assert_eq!(ws.move_tab(0, 1), Some(()));
            assert_eq!(tab_order(&ws), vec![sid(2), sid(1), sid(3)]);
            assert_eq!(active_id(&ws), sid(3));
        }

        #[test]
        fn a_drop_lands_in_the_zone_it_falls_in_and_the_boundary_keeps_its_own() {
            // [1 2 3 | 4 5]: three unparked tabs, then two parked ones.
            let ws = strip(3, 2);
            assert_eq!(ws.lands_parked(0, 1), Some(false), "within unparked");
            assert_eq!(ws.lands_parked(0, 2), Some(false), "boundary, unparked");
            assert_eq!(ws.lands_parked(0, 3), Some(true), "into the parked zone");
            assert_eq!(ws.lands_parked(4, 3), Some(true), "boundary, parked");
            assert_eq!(ws.lands_parked(4, 2), Some(false), "into the unparked zone");
            assert_eq!(ws.lands_parked(3, 3), Some(true), "a parked tab held still");
            assert_eq!(
                ws.lands_parked(1, 1),
                Some(false),
                "an unparked tab held still"
            );
        }

        #[test]
        fn lands_parked_refuses_an_index_off_the_strip() {
            let ws = strip(2, 1);
            assert_eq!(ws.lands_parked(3, 0), None);
            assert_eq!(ws.lands_parked(0, 3), None);
        }

        #[test]
        fn lands_parked_does_not_panic_on_a_strip_out_of_zone_order() {
            // A parked tab ahead of an unparked one breaks the strip rules;
            // the predicate refuses rather than underflowing.
            let mut ws = strip(2, 0);
            ws.tabs[0].parked = true;
            assert_eq!(ws.lands_parked(1, 0), None);
        }

        /// The drop-zone rule stated by neighbours rather than by count: in
        /// the strip without the carried tab, a parked tab to the left of the
        /// drop parks it, an unparked one to the right unparks it, and a drop
        /// with neither keeps the tab's own zone.
        fn zone_by_neighbours(ws: &Workspace, from: usize, to: usize) -> bool {
            let mut others: Vec<bool> = ws.tabs.iter().map(Tab::is_parked).collect();
            let was_parked = others.remove(from);
            let left = to.checked_sub(1).and_then(|i| others.get(i).copied());
            let right = others.get(to).copied();
            match (left, right) {
                (Some(true), _) => true,
                (_, Some(false)) => false,
                _ => was_parked,
            }
        }

        proptest! {
            /// The predicate agrees with the rule stated independently by
            /// neighbours, and `move_tab` applying it keeps the strip rules.
            #[test]
            fn lands_parked_matches_the_neighbour_rule_and_move_tab_applies_it(
                unparked in 0usize..5,
                parked in 0usize..5,
                from in 0usize..10,
                to in 0usize..10,
            ) {
                let mut ws = strip(unparked, parked);
                let predicted = ws.lands_parked(from, to);
                let in_range = from < ws.tabs.len() && to < ws.tabs.len();
                prop_assert_eq!(predicted.is_some(), in_range);
                if let Some(parked) = predicted {
                    prop_assert_eq!(parked, zone_by_neighbours(&ws, from, to));
                    prop_assert_eq!(ws.move_tab(from, to), Some(()));
                    prop_assert_eq!(ws.tabs[to].is_parked(), parked);
                    prop_assert_eq!(strip_rules_hold(&ws), Ok(()));
                }
            }
        }

        #[derive(Debug, Clone)]
        enum Op {
            Open,
            Append,
            Park(usize),
            Unpark(usize),
            Activate(usize),
            Move(usize, usize),
            Close(usize),
            Reveal(u64),
        }

        fn op() -> impl Strategy<Value = Op> {
            prop_oneof![
                Just(Op::Open),
                Just(Op::Append),
                (0usize..8).prop_map(Op::Park),
                (0usize..8).prop_map(Op::Unpark),
                (0usize..8).prop_map(Op::Activate),
                (0usize..8, 0usize..8).prop_map(|(from, to)| Op::Move(from, to)),
                (0usize..8).prop_map(Op::Close),
                (1u64..12).prop_map(Op::Reveal),
            ]
        }

        fn apply(ws: &mut Workspace, op: &Op, last_id: &mut u64) {
            match *op {
                Op::Open => {
                    *last_id += 1;
                    ws.open(sid(*last_id), "opened");
                }
                Op::Append => {
                    *last_id += 1;
                    ws.append(sid(*last_id), "appended");
                }
                Op::Park(index) => {
                    ws.park(index);
                }
                Op::Unpark(index) => {
                    ws.unpark(index);
                }
                Op::Activate(index) => {
                    ws.activate(index);
                }
                Op::Move(from, to) => {
                    ws.move_tab(from, to);
                }
                Op::Close(index) => {
                    ws.close_tab(index);
                }
                Op::Reveal(id) => {
                    ws.reveal_pane_of(sid(id));
                }
            }
        }

        proptest! {
            #[test]
            fn every_mutator_keeps_parked_tabs_after_unparked_ones(ops in vec(op(), 0..40)) {
                let mut ws = strip(3, 2);
                let mut last_id = 5;
                for op in &ops {
                    apply(&mut ws, op, &mut last_id);
                    prop_assert_eq!(strip_rules_hold(&ws), Ok(()), "after {:?}", op);
                }
            }

            #[test]
            fn the_tab_cycle_never_lands_on_a_parked_tab(ops in vec(op(), 0..40)) {
                let mut ws = strip(3, 2);
                let mut last_id = 5;
                for op in &ops {
                    apply(&mut ws, op, &mut last_id);
                }
                for step in [Step::Next, Step::Prev] {
                    if let Some(index) = ws.cycled_tab(step) {
                        prop_assert!(!ws.tabs[index].is_parked(), "{:?} landed on {}", step, index);
                    }
                }
            }
        }
    }
}
