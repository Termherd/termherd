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
    pub title: String,
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
        });
        self.active = self.tabs.len() - 1;
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
    /// reports over OSC (#24). Returns `None` if no tab hosts the session,
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
        let active = self.active;
        // Read what we need first, releasing the borrow before any `self.*`.
        let (removed, is_root) = {
            let tab = self.tabs.get(active)?;
            let removed = match navigate(&tab.root, &tab.focus)? {
                Pane::Leaf(session) => *session,
                Pane::Split { .. } => return None,
            };
            (removed, tab.focus.is_empty())
        };
        if is_root {
            self.close_tab(active);
            return Some(removed);
        }
        let tab = self.tabs.get_mut(active)?;
        let branch = tab.focus.pop()?;
        let parent = navigate_mut(&mut tab.root, &tab.focus)?;
        // Replace the parent split with the sibling subtree (the other branch).
        let taken = std::mem::replace(parent, Pane::Leaf(removed));
        if let Pane::Split { a, b, .. } = taken {
            *parent = match branch {
                Branch::A => *b,
                Branch::B => *a,
            };
        }
        // Focus now points at the sibling subtree; descend to its first leaf.
        focus_first_leaf(&tab.root, &mut tab.focus);
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
}

/// Extend `focus` from the node it points at down to that subtree's first
/// (leftmost) leaf, so the focus path always ends on a leaf.
fn focus_first_leaf(root: &Pane, focus: &mut Vec<Branch>) {
    while let Some(Pane::Split { .. }) = navigate(root, focus) {
        focus.push(Branch::A);
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
        /// Regression guard for the method the number-row jump leans on
        /// (issue #26): `activate` selects an in-range index, rejects anything
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
}
