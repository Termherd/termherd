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
}
