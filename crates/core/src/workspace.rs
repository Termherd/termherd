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
}
