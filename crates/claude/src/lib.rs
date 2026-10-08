//! termherd-claude — Claude CLI format codec.
//!
//! Pure. No I/O. The ported domain knowledge — path encoding/derivation,
//! JSONL digest parsing, transition signals, OSC decoding. Everything here is
//! deterministic and property-testable.

pub mod color;
pub mod derive;
pub mod digest;
pub mod jsonl;
pub mod osc;
pub mod path;
pub mod session_file;
pub mod session_id;
