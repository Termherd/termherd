//! termherd-claude — Claude CLI format codec.
//!
//! Pure. No I/O. The ported domain knowledge — path encoding/derivation,
//! JSONL digest parsing, transition signals, OSC decoding. Everything here is
//! deterministic and property-testable.

pub mod derive;
pub mod digest;
pub mod osc;
pub mod path;
