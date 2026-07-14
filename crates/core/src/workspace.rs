//! Pane tree + tabs — pure data, no I/O.
//!
//! The riskiest feature in v0 (tabs + splits + focus) lives here and is fully
//! testable headless. See `docs/ARCHITECTURE.md` §6.

use std::num::NonZeroU64;

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
    /// The split orientation this move traverses, and whether it heads toward
    /// the B (right/bottom) branch.
    fn axis_toward_b(self) -> (SplitDir, bool) {
        match self {
            Direction::Right => (SplitDir::Vertical, true),
            Direction::Left => (SplitDir::Vertical, false),
            Direction::Down => (SplitDir::Horizontal, true),
            Direction::Up => (SplitDir::Horizontal, false),
        }
    }
}

/// One side of a split, used to address a pane along the tree path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch {
    A,
    B,
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
    /// The derived title — the scanned session digest / OSC title, updated by
    /// [`Workspace::set_session_title`]. Shown only when there is no manual
    /// override (see [`Tab::custom_title`] / [`Tab::display_title`]).
    pub title: String,
    /// A user-set name that overrides the derived [`title`](Self::title) and is
    /// never clobbered by a later derived/OSC update — the manual rename wins.
    /// `None` means "use the derived title"; a rename to blank reverts to it.
    pub custom_title: Option<String>,
}

impl Tab {
    /// Every session hosted by this tab, in left-to-right pane order. A tab
    /// without splits yields exactly one id; closing the tab kills them all.
    #[must_use]
    pub fn sessions(&self) -> Vec<SessionId> {
        let mut out = Vec::new();
        collect_leaves(&self.root, &mut out);
        out
    }

    /// The title to display: the manual override when set, else the derived
    /// title. This is the single read every surface (tab chip, close prompt,
    /// hover card) should use so the override is honoured everywhere.
    #[must_use]
    pub fn display_title(&self) -> &str {
        self.custom_title.as_deref().unwrap_or(&self.title)
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
        self.tabs.push(Tab {
            root: Pane::Leaf(session),
            focus: Vec::new(),
            title: title.into(),
            custom_title: None,
        });
        self.active = self.tabs.len() - 1;
    }

    /// Index of the tab hosting `session`, if any. A session lives in exactly
    /// one tab (possibly inside a split), so this lets the shell re-focus an
    /// already-open session instead of launching a duplicate. Also a stable
    /// handle for state that must survive tab reordering — an inline rename
    /// anchors on it rather than a positional index, which a reorder or a
    /// sibling close would shift.
    #[must_use]
    pub fn tab_of(&self, session: SessionId) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab| tab.sessions().contains(&session))
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

    /// Set the title of the tab hosting `session`, to follow the title Claude
    /// reports over OSC. Returns `None` if no tab hosts the session,
    /// leaving every title unchanged.
    pub fn set_session_title(
        &mut self,
        session: SessionId,
        title: impl Into<String>,
    ) -> Option<()> {
        let tab = self
            .tabs
            .iter_mut()
            .find(|tab| tab.sessions().contains(&session))?;
        tab.title = title.into();
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

    /// Make the tab at `index` active (FR5). Returns `None` if the index is
    /// out of range, leaving the active tab unchanged.
    pub fn activate(&mut self, index: usize) -> Option<()> {
        if index < self.tabs.len() {
            self.active = index;
            Some(())
        } else {
            None
        }
    }

    /// Move the tab at `from` so it lands at index `to` (FR5 drag-reorder),
    /// shifting the tabs in between. The active tab is kept pointing at
    /// the *same* tab, whether it was the moved one or merely shifted by it.
    /// Returns `None` (no change) if either index is out of range; `from == to`
    /// is a successful no-op.
    pub fn move_tab(&mut self, from: usize, to: usize) -> Option<()> {
        if from >= self.tabs.len() || to >= self.tabs.len() {
            return None;
        }
        if from != to {
            let tab = self.tabs.remove(from);
            self.tabs.insert(to, tab);
            self.active = shift_index(self.active, from, to);
        }
        Some(())
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
        if self.tabs.is_empty() {
            self.active = 0;
        } else if self.active > index {
            self.active -= 1;
        } else if self.active == index {
            self.active = self.active.min(self.tabs.len() - 1);
        }
        sessions
    }

    /// Session id of the focused pane in the active tab, if any.
    pub fn focused_session(&self) -> Option<SessionId> {
        let tab = self.tabs.get(self.active)?;
        match navigate(&tab.root, &tab.focus)? {
            Pane::Leaf(s) => Some(*s),
            Pane::Split { .. } => None,
        }
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
            let focused = match navigate(&tab.root, &tab.focus) {
                Some(Pane::Leaf(session)) => Some(*session),
                _ => None,
            };
            (removed, focused)
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
        self.cycle_focus(1)
    }

    /// Move focus to the previous pane in the active tab (FR6), wrapping.
    pub fn focus_prev(&mut self) -> Option<()> {
        self.cycle_focus(-1)
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

    /// Move pane focus one step in a spatial direction (FR6), cycling within
    /// the move's axis. It crosses the nearest ancestor split of that
    /// orientation into the sibling subtree, landing on the adjacent leaf; from
    /// the far edge it wraps to the opposite end. Returns `None` (focus
    /// unchanged) when no split of that orientation exists to move through.
    pub fn focus_dir(&mut self, dir: Direction) -> Option<()> {
        let (axis, toward_b) = dir.axis_toward_b();
        let tab = self.tabs.get_mut(self.active)?;
        let focus = tab.focus.clone();

        // A directional step: the nearest ancestor split of this axis where the
        // focused subtree sits on the branch we can leave in this direction.
        for i in (0..focus.len()).rev() {
            let parent = &focus[..i];
            let leaving = focus[i];
            let on_near_branch = leaving == if toward_b { Branch::A } else { Branch::B };
            if on_near_branch && ancestor_is(&tab.root, parent, axis) {
                let mut path = parent.to_vec();
                path.push(if toward_b { Branch::B } else { Branch::A });
                descend_edge(&tab.root, &mut path, toward_b);
                tab.focus = path;
                return Some(());
            }
        }

        // Already at the far edge → wrap to the opposite end of the outermost
        // split of this axis. No such split → nothing to move through.
        for i in 0..focus.len() {
            let parent = &focus[..i];
            if ancestor_is(&tab.root, parent, axis) {
                let mut path = parent.to_vec();
                path.push(if toward_b { Branch::A } else { Branch::B });
                descend_edge(&tab.root, &mut path, toward_b);
                tab.focus = path;
                return Some(());
            }
        }
        None
    }

    fn cycle_focus(&mut self, delta: i32) -> Option<()> {
        let tab = self.tabs.get_mut(self.active)?;
        let paths = leaf_paths(&tab.root);
        if paths.is_empty() {
            return None;
        }
        let current = paths.iter().position(|p| *p == tab.focus).unwrap_or(0);
        let len = paths.len() as i32;
        let next = (current as i32 + delta).rem_euclid(len) as usize;
        tab.focus = paths[next].clone();
        Some(())
    }

    /// The active-tab index after cycling by `delta`, wrapping around both ends
    /// (FR9 `NextTab` / `PrevTab`). `None` when there are no tabs, so the caller
    /// no-ops rather than switching to a tab that isn't there.
    #[must_use]
    pub fn cycled_tab(&self, delta: i32) -> Option<usize> {
        let count = self.tabs.len();
        if count == 0 {
            return None;
        }
        Some((self.active as i32 + delta).rem_euclid(count as i32) as usize)
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
    let after_remove = if i > from { i - 1 } else { i };
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
/// arrived from: moving toward B (right/down) lands on the sibling's leftmost /
/// topmost leaf, moving toward A on its rightmost / bottommost.
fn descend_edge(root: &Pane, path: &mut Vec<Branch>, toward_b: bool) {
    while let Some(Pane::Split { .. }) = navigate(root, path) {
        path.push(if toward_b { Branch::A } else { Branch::B });
    }
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
        let mut ws = Workspace::new();
        assert_eq!(ws.cycled_tab(1), None, "no tabs: nothing to cycle to");

        ws.open(sid(1), "a"); // tab 0
        ws.open(sid(2), "b"); // tab 1
        ws.open(sid(3), "c"); // tab 2, now active
        assert_eq!(ws.active, 2);

        // Forward from the last tab wraps round to the first.
        assert_eq!(ws.cycled_tab(1), Some(0));
        // Backward from the last tab steps to the middle.
        assert_eq!(ws.cycled_tab(-1), Some(1));
        // The query is pure — it computes the next index, it does not switch.
        assert_eq!(ws.active, 2, "cycled_tab must not mutate the active index");
    }

    #[test]
    fn set_session_title_relabels_the_hosting_tab_only() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "first");
        ws.open(sid(2), "second");
        // A split: tab 1 now hosts sid(2) and sid(3).
        ws.split(SplitDir::Vertical, sid(3));

        assert_eq!(ws.set_session_title(sid(1), "renamed"), Some(()));
        assert_eq!(ws.tabs[0].title, "renamed");
        // Any session in a split tab relabels that tab.
        assert_eq!(ws.set_session_title(sid(3), "split title"), Some(()));
        assert_eq!(ws.tabs[1].title, "split title");
        // The untouched tab keeps its title.
        assert_eq!(ws.tabs[0].title, "renamed");
        // An unknown session changes nothing.
        assert_eq!(ws.set_session_title(sid(99), "ghost"), None);
        assert_eq!(ws.tabs[0].title, "renamed");
        assert_eq!(ws.tabs[1].title, "split title");
    }

    #[test]
    fn focus_dir_moves_and_wraps_within_a_vertical_split() {
        // Two panes side by side (sid1 | sid2), focus on sid2 (the new pane).
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.split(SplitDir::Vertical, sid(2));
        assert_eq!(ws.focused_session(), Some(sid(2)));

        // Left steps to the pane on the left.
        assert_eq!(ws.focus_dir(Direction::Left), Some(()));
        assert_eq!(ws.focused_session(), Some(sid(1)));
        // Left again from the leftmost wraps to the rightmost (cyclic on axis).
        assert_eq!(ws.focus_dir(Direction::Left), Some(()));
        assert_eq!(ws.focused_session(), Some(sid(2)));
        // Right from the rightmost wraps back to the leftmost.
        assert_eq!(ws.focus_dir(Direction::Right), Some(()));
        assert_eq!(ws.focused_session(), Some(sid(1)));

        // Up/Down find no horizontal split to traverse: no-op.
        assert_eq!(ws.focus_dir(Direction::Up), None);
        assert_eq!(ws.focused_session(), Some(sid(1)));
    }

    #[test]
    fn focus_dir_traverses_the_matching_axis_in_a_nested_split() {
        // Left column sid1; right column stacked sid2 over sid3.
        //   Vertical( sid1 , Horizontal( sid2 , sid3 ) )
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.split(SplitDir::Vertical, sid(2)); // focus sid2 (right)
        ws.split(SplitDir::Horizontal, sid(3)); // focus sid3 (bottom-right)
        assert_eq!(ws.focused_session(), Some(sid(3)));

        // Up moves within the right column to sid2.
        assert_eq!(ws.focus_dir(Direction::Up), Some(()));
        assert_eq!(ws.focused_session(), Some(sid(2)));
        // Left crosses the outer vertical split to the left column.
        assert_eq!(ws.focus_dir(Direction::Left), Some(()));
        assert_eq!(ws.focused_session(), Some(sid(1)));
        // Right crosses back; the edge nearest the divider is sid2 (top-right).
        assert_eq!(ws.focus_dir(Direction::Right), Some(()));
        assert_eq!(ws.focused_session(), Some(sid(2)));
    }

    #[test]
    fn focus_pane_of_moves_focus_to_the_leaf_hosting_a_session() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        // A split moves focus to the new pane (sid(2)).
        ws.split(SplitDir::Vertical, sid(2));
        assert_eq!(ws.focused_session(), Some(sid(2)));

        // Clicking the other pane jumps focus to the leaf hosting sid(1).
        assert_eq!(ws.focus_pane_of(sid(1)), Some(()));
        assert_eq!(ws.focused_session(), Some(sid(1)));

        // A session absent from the active tab is a no-op, focus unchanged.
        assert_eq!(ws.focus_pane_of(sid(99)), None);
        assert_eq!(ws.focused_session(), Some(sid(1)));
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
        assert_eq!(ws.set_session_title(sid(1), "new derived"), Some(()));
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
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.open(sid(2), "b");
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
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.open(sid(2), "b");
        ws.open(sid(3), "c");
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
    fn nested_splits_keep_focus_on_newest_leaf() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.split(SplitDir::Horizontal, sid(2));
        ws.split(SplitDir::Vertical, sid(3));
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
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.open(sid(2), "b");
        assert_eq!(ws.active, 1);
        assert!(ws.activate(0).is_some());
        assert_eq!(ws.active, 0);
        assert_eq!(ws.focused_session(), Some(sid(1)));
        // Out of range leaves the active tab untouched.
        assert!(ws.activate(9).is_none());
        assert_eq!(ws.active, 0);
    }

    /// A workspace holding `count` single-session tabs, active on the last one.
    fn workspace_with_tabs(count: usize) -> Workspace {
        let mut ws = Workspace::new();
        for n in 0..count {
            ws.open(sid(n as u64 + 1), format!("t{n}"));
        }
        ws
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
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.split(SplitDir::Vertical, sid(2));
        ws.split(SplitDir::Horizontal, sid(3));
        assert_eq!(ws.tabs[0].sessions(), vec![sid(1), sid(2), sid(3)]);
    }

    #[test]
    fn close_tab_returns_its_sessions_and_keeps_active_valid() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.open(sid(2), "b");
        ws.open(sid(3), "c");
        // Active is the last tab (c). Closing an earlier tab shifts it down.
        assert_eq!(ws.active, 2);
        assert_eq!(ws.close_tab(0), vec![sid(1)]);
        assert_eq!(ws.tabs.len(), 2);
        assert_eq!(ws.active, 1);
        assert_eq!(ws.focused_session(), Some(sid(3)));
    }

    #[test]
    fn closing_the_active_last_tab_focuses_the_new_last() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.open(sid(2), "b");
        assert_eq!(ws.active, 1);
        assert_eq!(ws.close_tab(1), vec![sid(2)]);
        assert_eq!(ws.active, 0);
        assert_eq!(ws.focused_session(), Some(sid(1)));
    }

    #[test]
    fn close_tab_kills_all_sessions_in_a_split() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.split(SplitDir::Vertical, sid(2));
        assert_eq!(ws.close_tab(0), vec![sid(1), sid(2)]);
        assert!(ws.tabs.is_empty());
        assert_eq!(ws.active, 0);
    }

    #[test]
    fn tab_of_locates_the_hosting_tab_including_inside_a_split() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.open(sid(2), "b");
        ws.split(SplitDir::Vertical, sid(3)); // sid 3 joins tab "b" as a split
        assert_eq!(ws.tab_of(sid(1)), Some(0));
        assert_eq!(ws.tab_of(sid(2)), Some(1));
        assert_eq!(ws.tab_of(sid(3)), Some(1), "a split member maps to its tab");
        assert_eq!(ws.tab_of(sid(9)), None, "an unopened session has no tab");
    }

    #[test]
    fn close_tab_out_of_range_is_a_noop() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        assert!(ws.close_tab(5).is_empty());
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.active, 0);
    }

    #[test]
    fn close_focused_collapses_the_split_onto_the_sibling() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.split(SplitDir::Vertical, sid(2)); // focus on B (sid 2)
        // Closing the focused pane removes sid 2 and leaves just sid 1.
        assert_eq!(ws.close_focused(), Some(sid(2)));
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.tabs[0].root, Pane::Leaf(sid(1)));
        assert_eq!(ws.focused_session(), Some(sid(1)));
    }

    #[test]
    fn close_focused_on_a_single_pane_tab_closes_the_tab() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.open(sid(2), "b");
        // Active tab is the single-leaf "b"; closing its pane closes the tab.
        assert_eq!(ws.close_focused(), Some(sid(2)));
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.focused_session(), Some(sid(1)));
    }

    #[test]
    fn close_focused_in_a_nested_split_keeps_a_leaf_focused() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.split(SplitDir::Horizontal, sid(2));
        ws.split(SplitDir::Vertical, sid(3)); // focus B,B = sid 3
        assert_eq!(ws.close_focused(), Some(sid(3)));
        // The sibling (sid 2) takes the inner split's place and is focused.
        assert_eq!(ws.focused_session(), Some(sid(2)));
        assert_eq!(ws.tabs[0].sessions(), vec![sid(1), sid(2)]);
    }

    #[test]
    fn close_pane_of_collapses_an_unfocused_leaf_and_keeps_focus() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.split(SplitDir::Vertical, sid(2)); // focus on B (sid 2)
        // Closing the *unfocused* pane (sid 1) must not steal focus from sid 2.
        assert_eq!(ws.close_pane_of(sid(1)), Some(sid(1)));
        assert_eq!(ws.tabs[0].root, Pane::Leaf(sid(2)));
        assert_eq!(ws.focused_session(), Some(sid(2)));
    }

    #[test]
    fn close_pane_of_the_focused_leaf_moves_focus_to_the_sibling() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.split(SplitDir::Vertical, sid(2)); // focus on B (sid 2)
        assert_eq!(ws.close_pane_of(sid(2)), Some(sid(2)));
        assert_eq!(ws.tabs[0].root, Pane::Leaf(sid(1)));
        assert_eq!(ws.focused_session(), Some(sid(1)));
    }

    #[test]
    fn close_pane_of_reaches_into_an_inactive_tab_without_switching() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.split(SplitDir::Vertical, sid(2));
        ws.open(sid(3), "b"); // active tab is now "b"
        assert_eq!(ws.close_pane_of(sid(2)), Some(sid(2)));
        assert_eq!(ws.tabs[0].root, Pane::Leaf(sid(1)));
        // The active tab and its focus are untouched.
        assert_eq!(ws.active, 1);
        assert_eq!(ws.focused_session(), Some(sid(3)));
    }

    #[test]
    fn close_pane_of_a_root_leaf_closes_the_whole_tab() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.open(sid(2), "b");
        assert_eq!(ws.close_pane_of(sid(1)), Some(sid(1)));
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.focused_session(), Some(sid(2)));
    }

    #[test]
    fn close_pane_of_an_unknown_session_is_a_noop() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        assert_eq!(ws.close_pane_of(sid(9)), None);
        assert_eq!(ws.tabs.len(), 1);
    }

    #[test]
    fn focus_next_and_prev_cycle_through_panes() {
        let mut ws = Workspace::new();
        ws.open(sid(1), "a");
        ws.split(SplitDir::Vertical, sid(2));
        ws.split(SplitDir::Vertical, sid(3)); // leaves in order: 1, 2, 3; focus on 3
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

    /// The session ids hosted by each tab, in tab order — a stable identity to
    /// assert reordering against.
    fn tab_order(ws: &Workspace) -> Vec<SessionId> {
        ws.tabs.iter().map(|t| t.sessions()[0]).collect()
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
}
