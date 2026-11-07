//! What the walk may enter and read under the projects root. A leaf: `walk`
//! and `derive` both ask it, so the two can never disagree on which entry is
//! a folder, which is a transcript, or how much of one is read.

use std::fs;
use std::path::Path;

/// Whether the walk descends into `path`.
pub(crate) fn is_dir(path: &Path) -> bool {
    path.is_dir()
}

/// Whether the walk treats `path` as a file it may read.
pub(crate) fn is_file(path: &Path) -> bool {
    path.is_file()
}

/// The text of the transcript at `path`, or `None` when it cannot be read.
pub(crate) fn read_transcript(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}
