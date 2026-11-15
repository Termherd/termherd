//! `cargo run -p termherd-scan --example dev_fixture -- <root>`: write the dev
//! instance's Claude fixture under `<root>` (see `fixture.rs`), dated from now.
//! Point `CLAUDE_CONFIG_DIR` at `<root>/claude`; `just dev-instance` does both.

mod fixture;

use std::path::PathBuf;
use std::time::SystemTime;

fn main() -> std::io::Result<()> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "usage: dev_fixture <root>",
            )
        })?;
    fixture::write(&root, SystemTime::now())?;
    Ok(())
}
