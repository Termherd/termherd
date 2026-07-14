//! GIF screencast record state machine (`F-capture`, rung 2). `core` owns the
//! idle→recording transitions and the frame cap; the encoder lives in `app`.

use super::*;

impl App {
    /// Start or stop the GIF screencast. Starting from idle enters the
    /// recording state and asks the app to open its encoder; a zero `max_frames`
    /// is a no-op (nothing to record). Stopping finalises the GIF when frames
    /// were captured, or cancels it outright when none were (the zero-frame
    /// guard — a start immediately followed by a stop writes no file).
    pub(super) fn toggle_record(&mut self, max_frames: u32) -> Vec<Effect> {
        match self.recording.take() {
            None => {
                if max_frames == 0 {
                    return Vec::new();
                }
                self.recording = Some(Recording {
                    frames: 0,
                    max_frames,
                });
                vec![Effect::StartRecording]
            }
            Some(recording) if recording.frames > 0 => {
                vec![Effect::FinishRecording { capped: false }]
            }
            Some(_) => vec![Effect::CancelRecording],
        }
    }

    /// One frame of the screencast: count it and ask the app to capture
    /// it, then auto-stop once the cap is reached. A tick while not recording is
    /// a silent no-op (a stray timer beat after a stop).
    pub(super) fn record_tick(&mut self) -> Vec<Effect> {
        let Some(recording) = self.recording.as_mut() else {
            return Vec::new();
        };
        recording.frames += 1;
        let mut effects = vec![Effect::CaptureFrame];
        if recording.frames >= recording.max_frames {
            self.recording = None;
            effects.push(Effect::FinishRecording { capped: true });
        }
        effects
    }

    /// Whether a GIF screencast is in progress — the app gates its frame
    /// timer subscription on this.
    #[must_use]
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Screencast progress as `(frames captured, frame cap)` while recording, or
    /// `None` when idle. The shell renders it as the `● REC n/cap`
    /// indicator so the recording state — and how close it is to auto-stop — is
    /// visible at a glance.
    #[must_use]
    pub fn recording_progress(&self) -> Option<(u32, u32)> {
        self.recording.map(|r| (r.frames, r.max_frames))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_record_starts_then_a_manual_toggle_finishes() {
        let mut app = App::new();
        assert!(!app.is_recording());

        let start = app.apply(Event::ToggleRecord { max_frames: 10 });
        assert!(matches!(start.as_slice(), [Effect::StartRecording]));
        assert!(app.is_recording());

        // Capture a couple of frames, then stop by hand.
        assert!(matches!(
            app.apply(Event::RecordTick).as_slice(),
            [Effect::CaptureFrame]
        ));
        assert!(matches!(
            app.apply(Event::RecordTick).as_slice(),
            [Effect::CaptureFrame]
        ));
        let stop = app.apply(Event::ToggleRecord { max_frames: 10 });
        assert!(matches!(
            stop.as_slice(),
            [Effect::FinishRecording { capped: false }]
        ));
        assert!(!app.is_recording());
    }

    #[test]
    fn the_frame_cap_auto_stops_the_recording() {
        let mut app = App::new();
        app.apply(Event::ToggleRecord { max_frames: 3 });

        // The first two ticks just capture; the third hits the cap and finishes.
        assert!(matches!(
            app.apply(Event::RecordTick).as_slice(),
            [Effect::CaptureFrame]
        ));
        assert!(matches!(
            app.apply(Event::RecordTick).as_slice(),
            [Effect::CaptureFrame]
        ));
        let last = app.apply(Event::RecordTick);
        assert!(
            matches!(
                last.as_slice(),
                [
                    Effect::CaptureFrame,
                    Effect::FinishRecording { capped: true }
                ]
            ),
            "the cap frame is captured, then the recording finishes, got {last:?}"
        );
        assert!(!app.is_recording(), "the cap auto-stops the recording");

        // A stray tick after the auto-stop is a silent no-op.
        assert!(app.apply(Event::RecordTick).is_empty());
    }

    #[test]
    fn stopping_before_any_frame_cancels_without_writing() {
        // The zero-frame guard: start then immediately stop → no file.
        let mut app = App::new();
        app.apply(Event::ToggleRecord { max_frames: 10 });
        let stop = app.apply(Event::ToggleRecord { max_frames: 10 });
        assert!(matches!(stop.as_slice(), [Effect::CancelRecording]));
        assert!(!app.is_recording());
    }

    #[test]
    fn a_zero_cap_record_is_a_noop() {
        let mut app = App::new();
        assert!(app.apply(Event::ToggleRecord { max_frames: 0 }).is_empty());
        assert!(!app.is_recording());
    }

    #[test]
    fn a_record_tick_while_idle_is_a_noop() {
        let mut app = App::new();
        assert!(app.apply(Event::RecordTick).is_empty());
        assert!(!app.is_recording());
    }

    #[test]
    fn recording_progress_tracks_frames_against_the_cap() {
        let mut app = App::new();
        assert_eq!(app.recording_progress(), None, "idle has no progress");

        app.apply(Event::ToggleRecord { max_frames: 3 });
        assert_eq!(app.recording_progress(), Some((0, 3)), "starts at 0/cap");

        app.apply(Event::RecordTick);
        assert_eq!(app.recording_progress(), Some((1, 3)));

        // The cap tick finishes the recording, so progress clears.
        app.apply(Event::RecordTick);
        app.apply(Event::RecordTick);
        assert_eq!(
            app.recording_progress(),
            None,
            "cleared once the cap stops it"
        );
    }

    proptest::proptest! {
        /// For any cap ≥ 1, exactly `max_frames` ticks capture `max_frames`
        /// frames and produce exactly one `FinishRecording`, leaving the app
        /// idle — and `apply` never panics (Q5).
        #[test]
        fn a_recording_captures_exactly_its_cap_then_finishes(max_frames in 1u32..200) {
            let mut app = App::new();
            app.apply(Event::ToggleRecord { max_frames });

            let mut captured = 0u32;
            let mut finishes = 0u32;
            for _ in 0..max_frames {
                for effect in app.apply(Event::RecordTick) {
                    match effect {
                        Effect::CaptureFrame => captured += 1,
                        Effect::FinishRecording { .. } => finishes += 1,
                        other => proptest::prop_assert!(false, "unexpected {:?}", other),
                    }
                }
            }
            proptest::prop_assert_eq!(captured, max_frames);
            proptest::prop_assert_eq!(finishes, 1);
            proptest::prop_assert!(!app.is_recording());
        }
    }
}
