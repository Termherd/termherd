//! Headless `App` — pure state machine over `Event`/`Effect`.
//!
//! The quality keystone (see `docs/ARCHITECTURE.md` §5). M0 is a placeholder;
//! events and effects grow incrementally with each milestone.

use crate::workspace::Workspace;

#[derive(Debug, Default)]
pub struct App {
    pub workspace: Workspace,
}

#[derive(Debug, Clone)]
pub enum Event {
    Tick,
}

#[derive(Debug, Clone)]
pub enum Effect {
    None,
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply an event, returning the effects the runtime must carry out.
    /// **Pure**: no I/O, no clock, no panic.
    pub fn apply(&mut self, _event: Event) -> Vec<Effect> {
        Vec::new()
    }
}
