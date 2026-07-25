//! The bridge-serving seam: how the shell answers one external [`Request`].
//!
//! Split from the shell's state machine because answering is not one thing. A
//! read answers off the state the shell already owns; a mutation needs `&mut`
//! and the effect executor, so it cannot go through the read-only responder.
//! Keeping that fork here — rather than inline in `update` — leaves one
//! auditable place where an external caller meets `core::App`.

use iced::Task;

use super::bridge::{self, ReplyPort, Request};
use super::{Message, Shell};

impl Shell {
    /// Answer one bridge request and return any async follow-up it needs.
    pub(super) fn serve(&mut self, request: Request, reply: ReplyPort) -> Task<Message> {
        match request {
            // Actions mutate, so they can't answer off a `&App`: apply them and
            // perform the effects here, where the shell owns both.
            Request::Act(action) => {
                let (outcome, task) = self.perform_action(action);
                reply.answer(bridge::Reply::Acted(outcome));
                task
            }
            // The read requests answer straight from owned state.
            read => {
                let inputs = self.snapshot_inputs(&read);
                reply.answer(bridge::respond(&self.core, &read, &inputs));
                Task::none()
            }
        }
    }
}
