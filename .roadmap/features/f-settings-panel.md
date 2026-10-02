+++
id = "F-settings-panel"
type = "feature"
area = ["workspace"]
status = "done"
target = ["Should"]
+++

An in-app settings panel — appearance first, applied live and saved on pick.

The ⚙ in the sidebar header, or `mod+,` (`open-settings`), opens a modal
panel over the workspace: the chrome theme (`dark`, `light`, and the four
presets named after the terminal schemes) and the terminal scheme. A pick
applies at once — the chrome on the next frame, every running terminal through
a shared palette the PTY manager repaints — and writes only its own key to
`settings.json`, leaving the rest of the file as written. This is the live
reload [F-terminal-palette](#f-terminal-palette) deferred, and the in-app
panel [F-settings](#f-settings) promised. A running Claude session keeps the
light/dark look it chose from its startup colour query until it restarts.
Other settings stay file-only for now.
