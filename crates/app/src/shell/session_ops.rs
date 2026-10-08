//! Tab lifecycle (FR5) and the quit path: closing/activating/cycling tabs, the
//! window-event router, and the single quit convergence point. Split from the
//! shell's state machine so the destructive close/quit flows — each guarded by
//! a confirmation policy — live in one place.

use iced::{Task, window};

use termherd_core::Effect;
use termherd_core::workspace::{SessionId, index_after_removal};

use super::{Focus, Message, Shell, TabDrag};

impl Shell {
    /// Handle a request to close the tab at `index`. The configured `close.tab`
    /// policy decides: arm the confirmation bar or close straight away.
    /// `confirmWhenActive` (the default) keys off the core
    /// `tab_has_running_process` predicate — an idle tab has nothing to lose and
    /// closes silently, a running one confirms; `alwaysConfirm` / `noConfirmation`
    /// override that. No-op for an out-of-range index, so a stale request can
    /// never close the wrong tab.
    /// `None` when the request was refused rather than acted on — a prompt is
    /// already up, or the index names no tab — so a caller reporting what
    /// happened does not claim a close that never started.
    pub(super) fn request_close(&mut self, index: usize) -> Option<Task<Message>> {
        // A pending confirmation owns the interaction (like the keyboard in
        // `on_key`): while one is up, ignore a close request for another tab so
        // it can't silently close that tab and drop the unanswered prompt.
        if self.closing.is_some() {
            return None;
        }
        if index >= self.core.workspace.tabs.len() {
            return None;
        }
        if self
            .close_confirm
            .tab
            .confirms(self.core.tab_has_running_process(index))
        {
            self.closing = Some(index);
            // Arming the prompt *is* the action: it happened, and the caller is
            // told which prompt now owns the keyboard by its next press.
            Some(Task::none())
        } else {
            Some(self.close_tab(index))
        }
    }

    /// Close the tab at `index`, killing its session(s) (FR5). Reached only
    /// after the confirmation is accepted: the close button and the
    /// `CloseFocused` keymap action both arm `closing` first.
    pub(super) fn close_tab(&mut self, index: usize) -> Task<Message> {
        self.closing = None;
        // Capture the sessions about to die so their cached screens don't
        // outlive them in the shell.
        let dying = self
            .core
            .workspace
            .tabs
            .get(index)
            .map(|tab| tab.sessions())
            .unwrap_or_default();
        let effects = self.core.apply(termherd_core::Event::CloseTab(index));
        for id in dying {
            self.screens.remove(&id);
        }
        let kill = self.perform(effects);
        Task::batch([kill, self.resize_panes()])
    }

    /// Where `session`'s pane sits now, captured just before `core` closes it,
    /// so the shell can follow the strip afterwards.
    pub(super) fn vanishing_pane(&self, session: SessionId) -> VanishingPane {
        let workspace = &self.core.workspace;
        let tab = workspace.tab_of(session);
        VanishingPane {
            session,
            tab,
            lone: tab
                .and_then(|index| workspace.tabs.get(index))
                .is_some_and(|tab| tab.sessions().len() == 1),
            active: workspace.active,
        }
    }

    /// The shell-side follow-up to a pane `core` just closed. Its cached screen
    /// goes, as `close_tab` drops it. The UI state keyed by tab position — a
    /// pending close prompt, a tab drag — shifts with the strip when the pane
    /// took its tab with it, and is dropped only when it named that very tab.
    /// A pending tab rename anchored on the pane moves to the first pane left
    /// in the tab. Only a tab whose layout changed is resized: the active one
    /// when the pane was in it, or the tab behind it whose split collapsed.
    pub(super) fn after_pane_vanished(&mut self, pane: VanishingPane) -> Task<Message> {
        self.screens.remove(&pane.session);
        let survivor = pane.tab.filter(|_| !pane.lone);
        if let Some(removed) = pane.tab.filter(|_| pane.lone) {
            self.closing = self
                .closing
                .and_then(|index| index_after_removal(index, removed));
            self.tab_drag = self.tab_drag.and_then(|drag| drag.after_removal(removed));
        }
        if let Some((anchor, buffer)) = self.tab_rename.take() {
            self.tab_rename = if anchor == pane.session {
                self.first_session_of(survivor).map(|heir| (heir, buffer))
            } else {
                Some((anchor, buffer))
            };
        }
        match pane.tab {
            Some(tab) if tab == pane.active => self.resize_panes(),
            Some(tab) if !pane.lone => self.resize_tab(tab),
            _ => Task::none(),
        }
    }

    /// Close the focused pane once `prelude` (the reveal that focused it, if
    /// any) has been performed, then follow the strip as for any pane `core`
    /// closes. A lone pane takes its tab with it.
    pub(super) fn close_focused_pane_after(&mut self, mut prelude: Vec<Effect>) -> Task<Message> {
        let vanishing = self
            .core
            .workspace
            .focused_session()
            .map(|id| self.vanishing_pane(id));
        prelude.extend(self.core.apply(termherd_core::Event::CloseFocusedPane));
        let kill = self.perform(prelude);
        let follow = vanishing.map_or_else(Task::none, |pane| self.after_pane_vanished(pane));
        Task::batch([kill, follow])
    }

    /// The first pane of the tab at `index`, when there is one.
    fn first_session_of(&self, index: Option<usize>) -> Option<SessionId> {
        let tab = self.core.workspace.tabs.get(index?)?;
        tab.sessions().first().copied()
    }

    /// Switch to the tab at `index` and return focus to the terminal. Switching
    /// drops any pending confirmation. An out-of-range index is a
    /// silent no-op in `core`, so a number key with no matching tab does
    /// nothing.
    pub(super) fn activate_tab(&mut self, index: usize) -> Task<Message> {
        let effects = self.core.apply(termherd_core::Event::ActivateTab(index));
        self.focus = Focus::Terminal;
        self.closing = None;
        self.archiving = None;
        Task::batch([self.perform(effects), self.resize_panes()])
    }

    /// Switch the active tab by `delta`, wrapping around (FR9 `NextTab` /
    /// `PrevTab`). `None` when nothing is open, so a caller learns the cycle had
    /// nowhere to go instead of being told the tab changed.
    pub(super) fn cycle_tab(&mut self, delta: i32) -> Option<Task<Message>> {
        let next = self.core.workspace.cycled_tab(delta)?;
        Some(self.activate_tab(next))
    }

    pub(super) fn on_window_event(
        &mut self,
        id: window::Id,
        event: window::Event,
    ) -> Task<Message> {
        match event {
            window::Event::Opened { .. } => {
                // Reroute the macOS menu Quit item (and ⌘Q) through the iced
                // runtime. Done here, not in the boot closure: iced constructs
                // the app state *before* `run_app`, so the boot closure runs
                // ahead of winit's `applicationDidFinishLaunching` (where the
                // default menu is installed). By the time the window is `Opened`
                // the event loop is running and the menu exists, and we are on
                // the main thread. Fires once (single window); no-op on other
                // platforms.
                #[cfg(target_os = "macos")]
                match objc2_foundation::MainThreadMarker::new() {
                    Some(mtm) => crate::macos::route_quit_through_close(mtm),
                    // We expect to be on the main thread here; if not, skipping
                    // would silently leave Cmd+Q on AppKit's hard-kill
                    // `terminate:` with no trace explaining why. Log it.
                    None => tracing::warn!(
                        "window Opened off the main thread; Cmd+Q stays on AppKit terminate:"
                    ),
                }
                Task::none()
            }
            window::Event::Moved(position) => {
                self.bounds.x = Some(position.x);
                self.bounds.y = Some(position.y);
                Task::none()
            }
            window::Event::Resized(size) => {
                self.bounds.width = size.width;
                self.bounds.height = size.height;
                self.resize_panes()
            }
            window::Event::CloseRequested => {
                self.bounds.save();
                self.request_quit(id)
            }
            window::Event::Focused => {
                let effects = self
                    .core
                    .apply(termherd_core::Event::WindowFocusChanged(true));
                self.perform(effects)
            }
            window::Event::Unfocused => {
                // The modifier release can't reach an unfocused window (e.g.
                // Ctrl let go while the browser a link click opened is in
                // front), so treat it as released; winit re-reports the live
                // modifiers when focus returns.
                self.link_modifier = false;
                self.shift_modifier = false;
                // Same for a tab drag's release: committing it on some later,
                // unrelated release would drop the tab where nobody aimed it.
                self.tab_drag = None;
                let effects = self
                    .core
                    .apply(termherd_core::Event::WindowFocusChanged(false));
                self.perform(effects)
            }
            // A *folder* dropped on the window adds it to the sidebar
            // (`F-repo-add`) — the same destination as the `+` picker, reached
            // without a dialog. A dropped **file** is ignored: `FileDropped`
            // carries no cursor position (iced 0.14), so the drop cannot be
            // confined to the sidebar, and dragging a file onto a terminal is
            // a thing people do — it must not silently declare its directory.
            window::Event::FileDropped(path) if path.is_dir() => {
                self.declare_repo(Some(&path), super::repos::RepoGesture::Drop)
            }
            window::Event::FileDropped(path) => {
                tracing::debug!(path = %path.display(), "dropped file ignored; drop a folder to add a repo");
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The single convergence point for every way the user can quit TermHerd.
    /// All three macOS triggers — the window-close button, the menu Quit item,
    /// and Cmd+Q — arrive here as a `CloseRequested` window event: the menu
    /// Quit action is repointed from AppKit's `terminate:` to `performClose:`
    /// at startup (`crate::macos`), so it routes through winit's
    /// `windowShouldClose:` like the close button instead of terminating the
    /// process out from under us. Keeping one seam is the structural fix — a
    /// second, unguarded quit path is exactly the defect this prevents.
    ///
    /// A quit hard-kills every live session's foreground process. Whether it
    /// confirms first is the configured app policy: `confirmWhenActive` (the
    /// default) confirms only while some session is still running work — the
    /// core `any_running_process` predicate, the app-wide sibling of the one the
    /// tab close uses — so an all-idle app quits silently; `alwaysConfirm` /
    /// `noConfirmation` override that. `iced::exit` (not `window::close`) is what
    /// actually ends the process: on macOS winit cancels the OS terminate and
    /// `exit_on_close_request(false)` keeps the runtime alive, so a mere window
    /// close would survive.
    pub(super) fn request_quit(&mut self, id: window::Id) -> Task<Message> {
        if self
            .close_confirm
            .app
            .confirms(self.core.any_running_process())
        {
            self.closing_window = Some(id);
            Task::none()
        } else {
            tracing::info!("quit needs no confirmation; exiting");
            self.exiting = true;
            iced::exit()
        }
    }

    /// Whether a quit is awaiting confirmation (the modal is up).
    pub(super) fn quit_pending(&self) -> bool {
        self.closing_window.is_some()
    }
}

/// A pane's place in the strip just before `core` closed it: its tab (when one
/// hosted it), whether it was that tab's only pane — so the tab went with it —
/// and which tab was active.
#[derive(Debug, Clone, Copy)]
pub(super) struct VanishingPane {
    session: SessionId,
    tab: Option<usize>,
    lone: bool,
    active: usize,
}

impl TabDrag {
    /// The drag once the tab at `removed` is gone: it ends when it was the
    /// dragged tab, and stops aiming at a slot that vanished under the pointer,
    /// so a release there is a plain click rather than a move.
    fn after_removal(self, removed: usize) -> Option<Self> {
        let from = index_after_removal(self.from, removed)?;
        let over = index_after_removal(self.over, removed).unwrap_or(from);
        Some(Self { from, over })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_over_a_vanished_slot_becomes_a_click() {
        let drag = TabDrag { from: 0, over: 2 };
        assert_eq!(drag.after_removal(2), Some(TabDrag { from: 0, over: 0 }));
        assert_eq!(drag.after_removal(0), None, "the dragged tab is gone");
    }
}
