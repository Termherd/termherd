+++
id = "F-url-scheme"
type = "feature"
area = ["workspace", "packaging"]
status = "done"
target = ["Could"]
+++

Open a tab from an HTML link: a `termherd://` URL scheme.

Shipped (#446): `termherd://open?repo=<absolute path>` shows the tab working
in the repo, or opens a shell there. A link is untrusted input: it never
writes to a terminal nor launches Claude, and `core::deeplink` refuses
anything its grammar does not name. A repo path is the target key because it
survives a restart, where the MCP `handle` does not.

Every launch drops its link into `~/.termherd/inbox/`, which the running
instance watches and drains: a second launch (Windows, Linux) hands it over
and exits, a cold start reads it at startup, and on macOS the `GetURL` Apple
Event lands in the same inbox. The installers declare the scheme through
`cargo-packager`'s `deep-link-protocols`; the AppImage registers nothing.

Open: `session/<resume_id>` as a second action, should a link need to name
one conversation. Discovery from outside overlaps
[F-mcp-attach](#f-mcp-attach).
