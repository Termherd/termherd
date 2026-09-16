//! Readings of the terminal mode that more than one leaf needs: the one place
//! a `TermMode` bit set is turned into a domain answer, so the input encoder's
//! gate, the pointer route in the terminal thread and the `Screen` snapshot
//! cannot disagree about what the application asked for.

use alacritty_terminal::term::TermMode;
use termherd_core::MouseReporting;

/// The mouse reporting the application negotiated — DECSET 1000 / 1002 / 1003
/// — or `None` when it reads no mouse. The widest bit wins: an application
/// raising the mode may leave a lower one set, and each rung includes the ones
/// below it.
#[must_use]
pub(crate) fn mouse_reporting(mode: TermMode) -> Option<MouseReporting> {
    if mode.contains(TermMode::MOUSE_MOTION) {
        Some(MouseReporting::Motion)
    } else if mode.contains(TermMode::MOUSE_DRAG) {
        Some(MouseReporting::Drag)
    } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
        Some(MouseReporting::Click)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_bits_read_as_the_reporting_ladder() {
        assert_eq!(mouse_reporting(TermMode::empty()), None);
        assert_eq!(
            mouse_reporting(TermMode::SGR_MOUSE),
            None,
            "SGR alone reports nothing"
        );
        assert_eq!(
            mouse_reporting(TermMode::MOUSE_REPORT_CLICK),
            Some(MouseReporting::Click)
        );
        assert_eq!(
            mouse_reporting(TermMode::MOUSE_DRAG),
            Some(MouseReporting::Drag)
        );
        assert_eq!(
            mouse_reporting(TermMode::MOUSE_MOTION),
            Some(MouseReporting::Motion)
        );
    }

    #[test]
    fn when_several_bits_are_set_the_widest_reporting_wins() {
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_MOTION;
        assert_eq!(mouse_reporting(mode), Some(MouseReporting::Motion));
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG;
        assert_eq!(mouse_reporting(mode), Some(MouseReporting::Drag));
    }
}
