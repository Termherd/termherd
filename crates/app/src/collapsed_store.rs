//! Sidebar fold-state persistence — load/save the set of folded project
//! paths at `~/.termherd/collapsed.json`. A file adapter owned by the shell,
//! like [`crate::metadata_store`]; `core` holds the domain set and never does
//! I/O. The file plumbing lives in [`crate::json_store`].

use std::collections::HashSet;

use crate::paths::DataDir;

/// Load the folded-project set; any problem (no file, bad JSON) yields an empty
/// set — fold state must never block startup.
#[must_use]
pub fn load(dir: &DataDir) -> HashSet<String> {
    crate::json_store::load_json(&dir.file(FILE))
}

/// Persist the folded-project set. Failures are logged, never fatal.
pub fn save(dir: &DataDir, collapsed: &HashSet<String>) {
    crate::json_store::save_json(&dir.file(FILE), collapsed);
}

/// The file's name within the data dir (PRD §7).
const FILE: &str = "collapsed.json";
