//! Sidebar fold-state persistence — load/save the set of folded project
//! paths at `~/.termherd/collapsed.json`. A file adapter owned by the shell,
//! like [`crate::metadata_store`]; `core` holds the domain set and never does
//! I/O. The file plumbing lives in [`crate::json_store`].

use std::collections::HashSet;

/// Load the folded-project set; any problem (no file, bad JSON) yields an empty
/// set — fold state must never block startup.
#[must_use]
pub fn load() -> HashSet<String> {
    crate::json_store::load_json(FILE)
}

/// Persist the folded-project set. Failures are logged, never fatal.
pub fn save(collapsed: &HashSet<String>) {
    crate::json_store::save_json(FILE, collapsed);
}

/// `~/.termherd/collapsed.json` — the app data dir from the PRD (§7).
const FILE: &str = "collapsed.json";
