+++
id = "F-capture"
type = "feature"
area = ["workspace"]
status = "todo"
target = ["Could"]
+++

Capture termherd along a fidelity ladder: debug dumps, promo, bug repros.

Capture termherd (screenshots / screencasts) along a fidelity ladder, for three
goals: **G1** dev/AI debug loop, **G2** promo & tutorial visuals, **G3**
bug-repro recordings (devs now, maybe end users later). Brainstorm:
`brainstorm/20260627-auto-capture-screenshots.md`. Grounding: termherd is an
iced 0.14 GUI, so it ships `window::screenshot()` (cross-platform, `png`
already a dep) and `iced_test::screenshot()` for headless CI; TTY recorders
(asciinema/VHS) only capture the inner terminal, not the GUI shell. Capture is
an `Event`→`Effect` (pure `core`, I/O in `app`), surviving the hexagonal
tightening. Ladder:

- **Rung 0+1 (G1) — shipped (#108)** (intrinsic quality): ⌘⇧S → `Event::Capture`
  → `Effect::Capture` → a JSON state+PTY-text dump *and* an iced PNG to
  `~/.termherd/captures/capture-<ts>.{json,png}` an AI reads by newest stamp.
  The cheap, on-thesis first slice.
- **Rung 2 (G3) — shipped (#124, #126)** (intrinsic quality): reshaped ✂️ by
  feature-torture (`.personal/feature-torture/reports/F-capture-rung2.md`)
  to **one dev-only GIF screencast** slice (⌘⇧R toggle, pure-Rust `gif`,
  screenshot-loop driven by the window's present clock (`window::frames()`,
  throttled to fps — #128, fixing the idle-window time-lapse), hard frame cap;
  record state machine pure in `core`, encoder on a dedicated thread in `app`).
  **In-app mp4 was cut** —
  `x264` is GPL (relicenses the MIT binary) and `openh264` compiles C via
  `build.rs` on all 3 CI legs, both breaking the no-FFI / MIT / no-`unsafe`
  posture; **G2 promo polish routes to external recorders**. Settings-
  configurable budget (fps/cap/scale) is a follow-up (#127).
- **Seeded demo-data mode — design-first:** fixtures of fake sessions for
  clean, reproducible captures. Force-multiplier for G2/G3, not a capture
  method; revisit when rung 2 comes forward.
