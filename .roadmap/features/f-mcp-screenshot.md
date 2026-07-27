+++
id = "F-mcp-screenshot"
type = "feature"
area = ["mcp", "workspace"]
status = "done"
target = ["Could"]
+++

The pixel rung: the window as a PNG, for what text cannot answer.

**The pixel rung.** A `screenshot` tool returning the window as PNG image
content, for the render / colour / glyph questions the text `snapshot` cannot
answer. The first bridge request whose answer comes from outside `core::App`:
the pixels arrive from an async iced `window::screenshot`, so the reply port
travels *inside* the screenshot task and answers from there — unlike a wait it
parks in no list, since nothing the shell will later observe decides it.
Payload is the constraint, not an afterthought: an unscaled retina window is
megabytes of base64, so `max_width` (default 1200, clamped 64–4096) bounds the
image, a total-pixel ceiling bounds the tall windows a width alone never
reaches, the frame is **never** upscaled, and the result reports the size
actually produced. Shrinking **averages** the covered box rather than picking
the nearest pixel: `Screenshot.size` is physical, so the default bound is a
~0.4× reduction on a retina display and nearest-neighbour aliases terminal
glyphs into noise — an image that cannot answer the question it was requested
for. Averaging costs ~40% more PNG (measured, 130 kB → 185 kB at 900px), which
is the trade the tool description names. The decision is a pure function of
`Option<Screenshot>` (`shot_reply`), so sizing, degradation and encoding are
all testable headlessly; a window-less run answers with the reason as a
*tool-level* error (which the caller reads, unlike an `ErrorData`) pointing at
`snapshot`. Tidy-first prerequisite: `target_dims` / `resample_nearest` (from
the recorder) and the PNG encoder (from the capture dump) moved into one pure
`app::image` module rather than being copied a third time. Depends on #212/#193.
**#196 + #229 + #215 are one capability in three parts** — drive the UI, see
the pixels, read the terminal — the loop that lets an agent *verify* a gesture
fix instead of only proposing it. With #229 shipped, #196 is the one that
remains
