//! The one effect executor (Q3): every `core.apply` result flows through
//! [`Shell::perform`], which carries each effect out against the adapters. The
//! PTY/file/OS sinks are fire-and-forget (failures logged, never fatal); the
//! capture and record effects carry an async window screenshot and dedicated
//! state, so `perform` delegates them to their owners rather than dropping them.
//! The OS handoffs live in the [`os`] submodule — the only `cfg(target_os)`
//! home in the app crate besides `crate::macos`.

mod os;

use std::time::SystemTime;

use iced::{Task, window};
use termherd_core::{CaptureDump, Effect};
use termherd_pty::Screen;

use super::{Message, Shell};
use os::{notify, open_url};

impl Shell {
    /// Carry out every effect `core` asked for, returning any async follow-up
    /// they need (a capture / record window screenshot).
    pub(super) fn perform(&mut self, effects: Vec<Effect>) -> Task<Message> {
        let mut task = Task::none();
        for effect in effects {
            task = Task::batch([task, self.perform_one(effect)]);
        }
        task
    }

    /// Carry out one effect. The PTY/file/OS sinks are quick (channel sends / a
    /// spawn / a file write); failures are logged, never fatal — a dead terminal
    /// must not take the app down (Q3) — and they yield no task. Capture and
    /// record own an async window screenshot and dedicated state, so they are
    /// delegated to their owners.
    fn perform_one(&mut self, effect: Effect) -> Task<Message> {
        let outcome = match effect {
            Effect::Spawn(spec) => self.pty.spawn(spec),
            Effect::Write { session, bytes } => self.pty.write(session, &bytes),
            Effect::Resize {
                session,
                cols,
                rows,
            } => self.pty.resize(session, cols, rows),
            Effect::Scroll { session, target } => self.pty.scroll(session, target),
            Effect::Select { session, op } => self.pty.select(session, op),
            Effect::CopyTerminalSelection { session } => self.pty.copy_selection(session),
            Effect::Kill(session) => self.pty.kill(session),
            // Metadata / fold state are file writes, not PTY calls.
            Effect::SaveMetadata(metadata) => {
                crate::metadata_store::save(&metadata);
                Ok(())
            }
            Effect::SaveCollapsed(collapsed) => {
                crate::collapsed_store::save(&collapsed);
                Ok(())
            }
            // Opening a link and a desktop notification are OS handoffs.
            Effect::OpenUrl(url) => open_url(&url),
            Effect::Notify { title, body } => notify(&title, &body),
            // Capture writes the dump and schedules the PNG; record drives the
            // encoder thread. Both return a task the loop above batches in.
            Effect::Capture(dump) => return self.capture_dump(dump),
            Effect::StartRecording
            | Effect::CaptureFrame
            | Effect::FinishRecording { .. }
            | Effect::CancelRecording => return self.record.run_one(effect),
        };
        if let Err(error) = outcome {
            tracing::warn!(%error, "pty effect failed");
        }
        Task::none()
    }

    /// The focused terminal's visible grid as text, for a capture. `None`
    /// when nothing is focused or its screen has not rendered yet — `core` then
    /// records a focus-less dump.
    pub(super) fn focused_pty_text(&self) -> Option<String> {
        let id = self.core.workspace.focused_session()?;
        self.screens.get(&id).map(Screen::text)
    }

    /// Capture the current state for the AI dev loop: hand `core` the focused
    /// terminal's text, then perform the returned effects — the `Effect::Capture`
    /// writes the JSON dump and schedules the PNG into `~/.termherd/captures/`.
    pub(super) fn capture(&mut self) -> Task<Message> {
        let focused_pty_text = self.focused_pty_text();
        let effects = self
            .core
            .apply(termherd_core::Event::Capture { focused_pty_text });
        self.perform(effects)
    }

    /// Resolve the captures dir and write the dump for an `Effect::Capture`. A
    /// no-op when no home directory is set.
    fn capture_dump(&self, dump: CaptureDump) -> Task<Message> {
        let Some(dir) = crate::capture::captures_dir() else {
            tracing::warn!("no home directory; capture skipped");
            return Task::none();
        };
        self.perform_capture(&dir, dump)
    }

    /// Write the rung-0 JSON dump into `dir` now and schedule the rung-1 PNG.
    /// Both share one timestamp so the pair is easy to find; the JSON is written
    /// synchronously (cheap), the PNG follows once iced returns the window
    /// screenshot. `dir` is a seam: production passes `~/.termherd/captures`,
    /// tests a throwaway. Any I/O failure is logged, never fatal — a missed
    /// capture must not take the app down.
    pub(super) fn perform_capture(
        &self,
        dir: &std::path::Path,
        dump: CaptureDump,
    ) -> Task<Message> {
        if let Err(error) = std::fs::create_dir_all(dir) {
            tracing::warn!(%error, "could not create captures dir; capture skipped");
            return Task::none();
        }
        let stamp = crate::capture::stamp(SystemTime::now());
        match crate::capture::write_dump(dir, &stamp, &dump) {
            Ok(path) => tracing::info!(path = %path.display(), "capture dump written"),
            Err(error) => tracing::warn!(%error, "could not write capture dump"),
        }
        let png_path = crate::capture::png_path(dir, &stamp);
        // Screenshot the live window (rung 1), then encode + write the PNG.
        // `Task::<Option>::and_then` only fires for `Some`, so a window-less run
        // simply skips the PNG and the JSON dump still stands.
        window::latest()
            .and_then(window::screenshot)
            .map(move |screenshot| Message::CaptureScreenshot {
                screenshot,
                png_path: png_path.clone(),
            })
    }
}
