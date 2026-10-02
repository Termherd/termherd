+++
id = "F-settings"
type = "feature"
area = ["workspace"]
status = "done"
target = ["Must"]
+++

A `settings.json` for shell, theme and window prefs — file-based for now.

Shell select, theme, window prefs (M3): `~/.termherd/settings.json` (serde,
defaults on missing/corrupt) carries a shell profile (program + args), injected
into the `PtyManager` so each session launches the chosen shell, and a GUI
theme (dark/light) wired to the iced chrome. (thin) Window bounds keep their
own `window.json` (FR12). The appearance is now also set live from
[F-settings-panel](#f-settings-panel); the rest stays file-based
