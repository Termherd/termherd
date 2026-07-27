+++
id = "F-auto-update"
type = "feature"
area = ["packaging"]
status = "todo"
target = ["Should"]
+++

Check for a new release from inside the app and apply it.

Never scoped beyond the name, and gated on
[F-packaging-ci](#f-packaging-ci): an update path can only be as trustworthy
as the signature on what it installs, and the bundles are unsigned today.
