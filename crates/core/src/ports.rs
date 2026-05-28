//! Ports — traits defining the boundary between the headless core and the
//! outside world. Adapters in sibling crates implement these.
//!
//! Stubs in M0; method signatures grow as adapters land (store in M1, pty in
//! M2, scan in M1). The dependency rule: `core` declares ports, never imports
//! adapters.

use std::time::SystemTime;

pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
}

/// Real signatures land with the `store` adapter in M1.
pub trait SessionStore: Send + Sync {}

/// Real signatures land with the `pty` adapter in M2.
pub trait PtyHost: Send + Sync {}

/// Real signatures land with the `scan` adapter in M1.
pub trait ProjectScanner: Send + Sync {}
