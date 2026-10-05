//! `settings.json`, live: the port that carries a setting beyond the shell's
//! own state (the terminals' palette, the next session's shell, the file),
//! and the reload that re-applies the file when it changes on disk — whoever
//! wrote it: a text editor, an MCP `set_option`, the settings panel itself.
//!
//! A reload goes through the same wide-parse-then-clamp path as startup, so a
//! bad value degrades exactly as it does there. A file that does not parse at
//! all — typically one an editor is halfway through writing — leaves the
//! running settings untouched: the app never falls back to defaults mid-run.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use iced::Task;
use iced::futures::{SinkExt, Stream, StreamExt};
use termherd_pty::{Palette, PtyManager};

use super::{Message, Shell};
use crate::settings::{AppearanceChange, Settings, save_appearance};

/// Quiet period before a burst of writes to the file triggers one reload — an
/// editor's save is often several events.
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(200);

/// Where a setting goes beyond the shell's own state. A port of its own rather
/// than [`termherd_core::ports::PtyHost`] methods: a palette is RGB and a shell
/// profile is adapter config, neither of which `core` sees. Injected so a shell
/// under test neither recolours real terminals nor rewrites the user's
/// `settings.json`.
pub(crate) trait SettingsSink: Send + Sync {
    /// Recolour every terminal, running and future.
    fn repaint(&self, palette: Palette);
    /// Launch this shell from the next session on.
    fn set_shell(&self, shell: Option<termherd_pty::Shell>);
    /// Persist one key the settings panel picked.
    fn persist(&self, change: &AppearanceChange);
}

/// The real sink: the PTY manager and `settings.json`.
pub(crate) struct LiveSettings(pub(crate) Arc<PtyManager>);

impl SettingsSink for LiveSettings {
    fn repaint(&self, palette: Palette) {
        self.0.set_palette(palette);
    }

    fn set_shell(&self, shell: Option<termherd_pty::Shell>) {
        self.0.set_shell(shell);
    }

    fn persist(&self, change: &AppearanceChange) {
        save_appearance(change);
    }
}

/// Goes nowhere — the double for a shell whose test does not look.
impl SettingsSink for () {
    fn repaint(&self, _palette: Palette) {}
    fn set_shell(&self, _shell: Option<termherd_pty::Shell>) {}
    fn persist(&self, _change: &AppearanceChange) {}
}

impl Shell {
    /// Re-read `settings.json` after it changed on disk and apply it. A file
    /// that does not parse changes nothing.
    pub(super) fn reload_settings(&mut self) -> Task<Message> {
        match Settings::reload() {
            Some(settings) => self.apply_settings(&settings),
            None => Task::none(),
        }
    }

    /// Bring every live setting in line with `settings`. Idempotent: applying
    /// the values already in force changes nothing visible, which is what
    /// makes the settings panel's own writes — echoed back by the watch —
    /// harmless.
    pub(super) fn apply_settings(&mut self, settings: &Settings) -> Task<Message> {
        self.appearance.theme = settings.theme;
        let palette = settings.palette();
        if palette != self.appearance.colors.to_palette() {
            self.settings_sink.repaint(palette);
        }
        self.appearance.colors = settings.terminal.colors.clone();
        self.keymap = settings.keymap();
        self.close_confirm = settings.close;
        self.gestures = settings.clipboard_gestures();
        self.record.set_config(settings.record_config());
        self.config = super::config_input(settings);
        self.settings_sink.set_shell(settings.shell_profile());

        let font_before = self.core.font_size();
        for event in [
            termherd_core::Event::SessionLimitLoaded(settings.session_limit()),
            termherd_core::Event::FontSizeLoaded(settings.font_size()),
            termherd_core::Event::OpenCommandLoaded(settings.open_command()),
        ] {
            self.core.apply(event);
        }
        // A new base size re-derives every grid, as a zoom does.
        if self.core.font_size() == font_before {
            Task::none()
        } else {
            self.resize_panes()
        }
    }
}

/// Watch `settings.json` and emit [`Message::SettingsFileChanged`] once per
/// burst of writes. If the watch cannot start, the file is simply read at the
/// next launch, as before (logged, not fatal).
// `&PathBuf` is imposed by `Subscription::run_with`, which passes `&D` to a
// plain fn pointer — `&Path` would not match `for<'a> fn(&'a D)`.
#[allow(clippy::ptr_arg)]
pub(super) fn settings_watch_stream(file: &PathBuf) -> impl Stream<Item = Message> + use<> {
    let file = file.clone();
    iced::stream::channel(
        4,
        |mut output: iced::futures::channel::mpsc::Sender<Message>| async move {
            let (tx, mut rx) = iced::futures::channel::mpsc::unbounded::<()>();
            match termherd_scan::watch_file(file, RELOAD_DEBOUNCE, move || {
                let _ = tx.unbounded_send(());
            }) {
                Ok(handle) => {
                    while rx.next().await.is_some() {
                        if output.send(Message::SettingsFileChanged).await.is_err() {
                            break;
                        }
                    }
                    drop(handle);
                }
                Err(error) => {
                    tracing::warn!(%error, "settings watch unavailable; changes apply at restart");
                    iced::futures::future::pending::<()>().await;
                }
            }
        },
    )
}
