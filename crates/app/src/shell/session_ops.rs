//! Tab lifecycle (FR5) and the quit path: closing/activating/cycling tabs, the
//! window-event router, and the single quit convergence point. Split from the
//! shell's state machine so the destructive close/quit flows — each guarded by
//! a confirmation policy — live in one place.

use iced::{Task, window};

use super::{Focus, Message, Shell};

impl Shell {
    /// Handle a request to close the tab at `index`. The configured `close.tab`
    /// policy decides: arm the confirmation bar or close straight away.
    /// `confirmWhenActive` (the default) keys off the core
    /// `tab_has_running_process` predicate — an idle tab has nothing to lose and
    /// closes silently, a running one confirms; `alwaysConfirm` / `noConfirmation`
    /// override that. No-op for an out-of-range index, so a stale request can
    /// never close the wrong tab.
    pub(super) fn request_close(&mut self, index: usize) -> Task<Message> {
        // A pending confirmation owns the interaction (like the keyboard in
        // `on_key`): while one is up, ignore a close request for another tab so
        // it can't silently close that tab and drop the unanswered prompt.
        if self.closing.is_some() {
            return Task::none();
        }
        if index >= self.core.workspace.tabs.len() {
            return Task::none();
        }
        if self
            .close_confirm
            .tab
            .confirms(self.core.tab_has_running_process(index))
        {
            self.closing = Some(index);
            Task::none()
        } else {
            self.close_tab(index)
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
    /// `PrevTab`). No-op when nothing is open.
    pub(super) fn cycle_tab(&mut self, delta: i32) -> Task<Message> {
        let Some(next) = self.core.workspace.cycled_tab(delta) else {
            return Task::none();
        };
        self.activate_tab(next)
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
                let effects = self
                    .core
                    .apply(termherd_core::Event::WindowFocusChanged(false));
                self.perform(effects)
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
