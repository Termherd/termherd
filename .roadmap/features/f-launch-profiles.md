+++
id = "F-launch-profiles"
type = "feature"
area = ["sessions"]
status = "todo"
target = ["Could"]
+++

Persistent per-project `--add-dir`, applied to fresh and resumed launches.

Parameterised session launch. **Tortured (✂️ reshape, feature-torture
`F-launch-profiles.md`).** The written framing (arbitrary flags: `--add-dir`,
`--model`, `--mcp-config`, launch profiles) mostly duplicates in-session slash
commands (`/add-dir`, `/model`) and what `--resume` restores. The one
non-redundant slice: **persistent per-project `--add-dir`, applied to both
fresh and `--resume` launches** — a multi-root repo opens already reading its
sibling dir, set once on the repo (flag composition `claude --resume {id}
--add-dir X` verified; `--add-dir` is variadic). Store it in the
`project_path`-keyed `~/.termherd/metadata.json` overlay (reuse #57), not a new
settings schema; ride the `Launch`-enum edit on `F-antigravity-sessions` (#162)
to touch it once. **Unblocked: #57's `repos` overlay shipped**, so `RepoMeta`
can now grow an `extra_dirs` field. Today `Launch::Claude` carries only `{
resume }` (`crates/core/src/app.rs`) and the command is *typed* into the shell
(`launch_command`, `crates/pty/src/lib.rs`), so the one real cost is
cross-shell path quoting (pwsh vs bash)
