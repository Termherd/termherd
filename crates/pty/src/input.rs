//! Terminal input byte protocol (FR4): keys, wheel and paste encoded to the
//! bytes a terminal expects. A pure, GUI-free leaf — the GUI translates its own
//! key/mouse events into these neutral types so the codec never depends on any
//! GUI crate. Depends only on `alacritty_terminal`'s [`TermMode`] to read what
//! mouse/scroll protocol the focused application negotiated.

use alacritty_terminal::term::TermMode;

/// The bytes a paste sends to the PTY (FR4). Newlines are normalised to the
/// carriage return the terminal expects for Enter; when the application has
/// enabled bracketed paste (see [`crate::Screen::bracketed_paste`]) the text is
/// wrapped in `ESC[200~`…`ESC[201~` so a multi-line paste arrives as one block
/// instead of submitting each line. Terminal byte protocol lives here, in the
/// terminal adapter, not in the GUI shell.
#[must_use]
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let mut out = Vec::with_capacity(normalized.len() + 12);
        out.extend_from_slice(b"\x1b[200~");
        out.extend_from_slice(normalized.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        normalized.into_bytes()
    }
}

/// A keyboard key in terms the terminal byte protocol cares about — a typed
/// character or one of the named keys that map to an escape sequence. This is
/// a framework-neutral boundary type: the GUI translates its own key events
/// into a `TermKey` (+ [`KeyMods`]) so this codec stays free of any GUI crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermKey {
    /// A character key; the layout-resolved text is passed to [`key_bytes`]
    /// alongside, so this carries only the base char for Ctrl/Alt handling.
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Space,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Delete,
    PageUp,
    PageDown,
}

/// The modifier keys held during a key press, as far as the byte protocol is
/// concerned (the platform Cmd/Super key never affects terminal bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyMods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// Translate a key press into the bytes a terminal expects (FR4): control
/// combinations, the named keys and (modifier-aware) cursor/editing sequences,
/// otherwise the layout-resolved `text`. Pure and GUI-free — the input byte
/// protocol lives here in the terminal adapter, next to [`paste_bytes`].
#[must_use]
pub fn key_bytes(key: TermKey, mods: KeyMods, text: Option<&str>) -> Option<Vec<u8>> {
    // Ctrl+<char> control bytes (Ctrl-A → 0x01, Ctrl-Space → NUL, …).
    if mods.ctrl
        && let TermKey::Char(ch) = key
    {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphabetic() {
            return Some(vec![(lower as u8 - b'a') + 1]);
        }
        match ch {
            ' ' => return Some(vec![0]),
            '[' => return Some(vec![27]),
            '\\' => return Some(vec![28]),
            ']' => return Some(vec![29]),
            _ => {}
        }
    }

    let modifier = modifier_param(mods);

    match key {
        // Enter carries its modifiers: Alt/Option+Enter emits `ESC CR` and
        // Shift+Enter a bare line feed, the two sequences Claude reads as
        // "insert a newline" instead of submitting; plain Enter still submits.
        TermKey::Enter => Some(if mods.alt {
            b"\x1b\r".to_vec()
        } else if mods.shift {
            b"\n".to_vec()
        } else {
            b"\r".to_vec()
        }),
        // Shift+Tab is back-tab (`CSI Z`); Claude uses it to cycle modes.
        TermKey::Tab if mods.shift => Some(b"\x1b[Z".to_vec()),
        TermKey::Tab => Some(b"\t".to_vec()),
        // Alt+Backspace is readline's delete-previous-word (`ESC DEL`).
        TermKey::Backspace => Some(if mods.alt {
            b"\x1b\x7f".to_vec()
        } else {
            b"\x7f".to_vec()
        }),
        TermKey::Escape => Some(b"\x1b".to_vec()),
        TermKey::Space => Some(b" ".to_vec()),
        // Cursor / navigation keys take a `1;<mod>` parameter when modified —
        // e.g. Ctrl+Right (`ESC[1;5C`) for word jump, Shift+Up (`ESC[1;2A`).
        TermKey::Up => Some(csi_letter(b'A', modifier)),
        TermKey::Down => Some(csi_letter(b'B', modifier)),
        TermKey::Right => Some(csi_letter(b'C', modifier)),
        TermKey::Left => Some(csi_letter(b'D', modifier)),
        TermKey::Home => Some(csi_letter(b'H', modifier)),
        TermKey::End => Some(csi_letter(b'F', modifier)),
        // `~`-terminated editing keys gain the same `;<mod>` parameter.
        TermKey::Delete => Some(csi_tilde(3, modifier)),
        TermKey::PageUp => Some(csi_tilde(5, modifier)),
        TermKey::PageDown => Some(csi_tilde(6, modifier)),
        // Alt+<char> (Meta) prefixes the character with `ESC`, the readline
        // convention for word-wise editing (Alt+B / Alt+F / Alt+D …). Limited
        // to ASCII alphanumerics so macOS Option-composed glyphs (e.g. Option+e
        // → "´") still fall through to the layout-resolved text below.
        TermKey::Char(ch) if mods.alt && !mods.ctrl && ch.is_ascii_alphanumeric() => {
            Some(vec![0x1b, ch as u8])
        }
        TermKey::Char(_) => text
            .filter(|t| !t.is_empty())
            .map(|t| t.as_bytes().to_vec()),
    }
}

/// Translate a wheel scroll into the bytes a full-screen application expects,
/// or `None` when the terminal isn't asking for wheel input — a normal screen
/// with no mouse mode, where the caller scrolls its own scrollback instead
/// This is why scrolling a Claude/vim/less TUI did nothing: those run in
/// the alternate screen with mouse reporting on, so the wheel must be *forwarded
/// to the app*, not applied to a (non-existent) scrollback.
///
/// `col`/`row` are 0-based pointer cell coordinates; `lines` is the signed wheel
/// delta (positive = up/back, negative = down/forward), matching the viewport
/// convention used elsewhere. Pure and GUI-free, next to [`key_bytes`].
#[must_use]
pub fn wheel_bytes(mode: TermMode, col: u16, row: u16, lines: i32) -> Option<Vec<u8>> {
    if lines == 0 {
        return None;
    }
    let count = lines.unsigned_abs() as usize;
    let up = lines > 0;

    // Mouse reporting: each wheel notch is a button-press event — button 64 up,
    // 65 down — at the pointer cell. SGR when negotiated (1006), else legacy X10.
    let mouse = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_MOTION | TermMode::MOUSE_DRAG;
    if mode.intersects(mouse) {
        let button: u8 = if up { 64 } else { 65 };
        // Mouse coordinates are 1-based.
        let (c, r) = (col + 1, row + 1);
        let mut out = Vec::new();
        for _ in 0..count {
            if mode.contains(TermMode::SGR_MOUSE) {
                out.extend_from_slice(format!("\x1b[<{button};{c};{r}M").as_bytes());
            } else {
                // ESC[M Cb Cx Cy, every value biased by 32 and capped at a byte.
                out.extend_from_slice(&[0x1b, b'[', b'M', x10(u16::from(button)), x10(c), x10(r)]);
            }
        }
        return Some(out);
    }

    // Alternate-scroll (1007) in the alt screen, no mouse mode: one cursor key
    // per line, in SS3 form when DECCKM (app-cursor) is on, else CSI.
    if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
        let final_byte = if up { b'A' } else { b'B' };
        // CSI form reuses the cursor-key encoder; DECCKM swaps it for SS3.
        let seq = if mode.contains(TermMode::APP_CURSOR) {
            vec![0x1b, b'O', final_byte]
        } else {
            csi_letter(final_byte, 1)
        };
        let mut out = Vec::with_capacity(seq.len() * count);
        for _ in 0..count {
            out.extend_from_slice(&seq);
        }
        return Some(out);
    }

    // Normal screen, no mouse mode: the caller scrolls its own scrollback.
    None
}

/// The legacy X10 mouse-coordinate byte: value biased by 32, saturating at the
/// 255 ceiling beyond which the unextended protocol can't address a cell.
fn x10(v: u16) -> u8 {
    v.saturating_add(32).min(255) as u8
}

/// The xterm modifier parameter (`1` = none): `+Shift`, `+Alt×2`, `+Ctrl×4`,
/// so Shift=2, Alt=3, Ctrl=5, Ctrl+Shift=6, … as terminals expect in
/// `CSI 1;<mod>` cursor sequences.
fn modifier_param(m: KeyMods) -> u8 {
    1 + (m.shift as u8) + (m.alt as u8) * 2 + (m.ctrl as u8) * 4
}

/// A letter-terminated cursor sequence (`A`=Up, `C`=Right, `H`=Home …):
/// `ESC[<final>` unmodified, `ESC[1;<mod><final>` when modified.
fn csi_letter(final_byte: u8, modifier: u8) -> Vec<u8> {
    if modifier <= 1 {
        vec![0x1b, b'[', final_byte]
    } else {
        format!("\x1b[1;{modifier}{}", final_byte as char).into_bytes()
    }
}

/// A `~`-terminated editing key (Delete=3, PageUp=5, PageDown=6):
/// `ESC[<n>~` unmodified, `ESC[<n>;<mod>~` when modified.
fn csi_tilde(n: u8, modifier: u8) -> Vec<u8> {
    if modifier <= 1 {
        format!("\x1b[{n}~").into_bytes()
    } else {
        format!("\x1b[{n};{modifier}~").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_normalises_newlines_and_wraps_when_bracketed() {
        // Plain paste: CRLF and LF collapse to the CR a terminal reads as Enter.
        assert_eq!(paste_bytes("a\r\nb\nc", false), b"a\rb\rc".to_vec());
        // Bracketed paste wraps the (normalised) text so it lands as one block.
        assert_eq!(
            paste_bytes("a\nb", true),
            b"\x1b[200~a\rb\x1b[201~".to_vec()
        );
    }

    const NONE: KeyMods = KeyMods {
        ctrl: false,
        alt: false,
        shift: false,
    };
    const CTRL: KeyMods = KeyMods {
        ctrl: true,
        alt: false,
        shift: false,
    };
    const ALT: KeyMods = KeyMods {
        ctrl: false,
        alt: true,
        shift: false,
    };
    const SHIFT: KeyMods = KeyMods {
        ctrl: false,
        alt: false,
        shift: true,
    };

    #[test]
    fn ctrl_letters_and_symbols_map_to_control_bytes() {
        assert_eq!(
            key_bytes(TermKey::Char('c'), CTRL, Some("c")),
            Some(vec![3])
        );
        assert_eq!(
            key_bytes(TermKey::Char('a'), CTRL, Some("a")),
            Some(vec![1])
        );
        // Ctrl+Space is NUL; the bracket family fills 0x1b–0x1d.
        assert_eq!(
            key_bytes(TermKey::Char(' '), CTRL, Some(" ")),
            Some(vec![0])
        );
        assert_eq!(
            key_bytes(TermKey::Char('['), CTRL, Some("[")),
            Some(vec![27])
        );
        assert_eq!(
            key_bytes(TermKey::Char(']'), CTRL, Some("]")),
            Some(vec![29])
        );
    }

    #[test]
    fn plain_named_keys_map_to_their_sequences() {
        assert_eq!(key_bytes(TermKey::Enter, NONE, None), Some(b"\r".to_vec()));
        assert_eq!(key_bytes(TermKey::Tab, NONE, None), Some(b"\t".to_vec()));
        assert_eq!(
            key_bytes(TermKey::Backspace, NONE, None),
            Some(b"\x7f".to_vec())
        );
        assert_eq!(
            key_bytes(TermKey::Escape, NONE, None),
            Some(b"\x1b".to_vec())
        );
        assert_eq!(key_bytes(TermKey::Space, NONE, None), Some(b" ".to_vec()));
        assert_eq!(key_bytes(TermKey::Up, NONE, None), Some(b"\x1b[A".to_vec()));
        assert_eq!(
            key_bytes(TermKey::Delete, NONE, None),
            Some(b"\x1b[3~".to_vec())
        );
    }

    #[test]
    fn enter_tab_and_backspace_carry_their_modifiers() {
        // Shift / Alt+Enter insert a newline instead of submitting.
        assert_eq!(key_bytes(TermKey::Enter, SHIFT, None), Some(b"\n".to_vec()));
        assert_eq!(
            key_bytes(TermKey::Enter, ALT, None),
            Some(b"\x1b\r".to_vec())
        );
        // Shift+Tab is back-tab; Alt+Backspace deletes the previous word.
        assert_eq!(
            key_bytes(TermKey::Tab, SHIFT, None),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            key_bytes(TermKey::Backspace, ALT, None),
            Some(b"\x1b\x7f".to_vec())
        );
    }

    #[test]
    fn modifier_param_follows_the_xterm_scheme() {
        assert_eq!(modifier_param(NONE), 1);
        assert_eq!(modifier_param(SHIFT), 2);
        assert_eq!(modifier_param(ALT), 3);
        assert_eq!(modifier_param(CTRL), 5);
        assert_eq!(
            modifier_param(KeyMods {
                ctrl: true,
                alt: false,
                shift: true
            }),
            6
        );
    }

    #[test]
    fn modified_cursor_and_tilde_keys_carry_a_csi_parameter() {
        // Ctrl+Right (word jump), Shift+Up (select), Ctrl+Home.
        assert_eq!(
            key_bytes(TermKey::Right, CTRL, None),
            Some(b"\x1b[1;5C".to_vec())
        );
        assert_eq!(
            key_bytes(TermKey::Up, SHIFT, None),
            Some(b"\x1b[1;2A".to_vec())
        );
        assert_eq!(
            key_bytes(TermKey::Home, CTRL, None),
            Some(b"\x1b[1;5H".to_vec())
        );
        // `~`-terminated keys gain the same parameter before the `~`.
        assert_eq!(
            key_bytes(TermKey::Delete, CTRL, None),
            Some(b"\x1b[3;5~".to_vec())
        );
        assert_eq!(
            key_bytes(TermKey::PageUp, CTRL, None),
            Some(b"\x1b[5;5~".to_vec())
        );
    }

    #[test]
    fn alt_letters_are_meta_prefixed_other_chars_send_text() {
        // Alt+B is readline word-motion: ESC then the letter.
        assert_eq!(
            key_bytes(TermKey::Char('b'), ALT, Some("b")),
            Some(vec![0x1b, b'b'])
        );
        // A macOS Option-composed glyph (non-ASCII) falls through to its text.
        assert_eq!(
            key_bytes(TermKey::Char('´'), ALT, Some("´")),
            Some("´".as_bytes().to_vec())
        );
        // A plain character sends its layout-resolved text; none -> nothing.
        assert_eq!(
            key_bytes(TermKey::Char('é'), NONE, Some("é")),
            Some("é".as_bytes().to_vec())
        );
        assert_eq!(key_bytes(TermKey::Char('x'), NONE, None), None);
    }

    // --- wheel forwarding for mouse-mode / alt-screen TUIs --------------

    const MOUSE: TermMode = TermMode::MOUSE_REPORT_CLICK;

    #[test]
    fn normal_screen_without_mouse_falls_back_to_scrollback() {
        // No mouse mode, no alt-screen: the caller should scroll its own
        // scrollback, so the codec declines (the plain-shell case, still works).
        assert_eq!(wheel_bytes(TermMode::empty(), 3, 4, 1), None);
    }

    #[test]
    fn zero_lines_is_a_noop() {
        assert_eq!(wheel_bytes(MOUSE | TermMode::SGR_MOUSE, 0, 0, 0), None);
    }

    #[test]
    fn sgr_mouse_wheel_up_is_button_64_at_the_pointer_cell() {
        // SGR: ESC[<b;col;row M, 1-based cell, wheel-up = button 64.
        let mode = MOUSE | TermMode::SGR_MOUSE;
        assert_eq!(wheel_bytes(mode, 4, 2, 1), Some(b"\x1b[<64;5;3M".to_vec()));
    }

    #[test]
    fn sgr_mouse_wheel_down_is_button_65() {
        let mode = MOUSE | TermMode::SGR_MOUSE;
        assert_eq!(wheel_bytes(mode, 4, 2, -1), Some(b"\x1b[<65;5;3M".to_vec()));
    }

    #[test]
    fn sgr_mouse_emits_one_event_per_line() {
        // A 3-line notch is three wheel events, not one.
        let mode = MOUSE | TermMode::SGR_MOUSE;
        assert_eq!(
            wheel_bytes(mode, 0, 0, 3),
            Some(b"\x1b[<64;1;1M\x1b[<64;1;1M\x1b[<64;1;1M".to_vec())
        );
    }

    #[test]
    fn legacy_mouse_wheel_uses_x10_encoding() {
        // Without SGR: ESC[M Cb Cx Cy, each value offset by 32. Button 64+32=96,
        // cell (0,0) → 1-based (1,1) → 33,33.
        assert_eq!(
            wheel_bytes(MOUSE, 0, 0, 1),
            Some(vec![0x1b, b'[', b'M', 96, 33, 33])
        );
    }

    #[test]
    fn alternate_scroll_sends_cursor_arrows_in_the_alt_screen() {
        // 1007 without mouse reporting: one cursor key per line, up = Up.
        let mode = TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL;
        assert_eq!(wheel_bytes(mode, 9, 9, 2), Some(b"\x1b[A\x1b[A".to_vec()));
        assert_eq!(wheel_bytes(mode, 9, 9, -1), Some(b"\x1b[B".to_vec()));
    }

    #[test]
    fn alternate_scroll_honours_app_cursor_mode() {
        // DECCKM on → SS3 form (ESC O A) instead of CSI (ESC [ A).
        let mode = TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL | TermMode::APP_CURSOR;
        assert_eq!(wheel_bytes(mode, 0, 0, 1), Some(b"\x1bOA".to_vec()));
    }

    #[test]
    fn alternate_scroll_outside_the_alt_screen_is_scrollback() {
        // 1007 set but on the normal screen → decline, scroll the scrollback.
        assert_eq!(wheel_bytes(TermMode::ALTERNATE_SCROLL, 0, 0, 1), None);
    }

    #[test]
    fn mouse_reporting_takes_priority_over_alternate_scroll() {
        // A TUI with both negotiated gets real wheel events, not arrows.
        let mode = MOUSE | TermMode::SGR_MOUSE | TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL;
        assert_eq!(wheel_bytes(mode, 0, 0, 1), Some(b"\x1b[<64;1;1M".to_vec()));
    }
}
