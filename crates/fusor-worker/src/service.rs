mod close;
mod spawn;

pub use close::Close;
pub use spawn::{Spawn, spawn};

use crate::{
    Job, Message, WorkerError,
    endpoint::Endpoint,
    job::{JobKind, Requirement, State, Target},
};
use fusor::OwnerHandle;
use std::{cell::RefCell, marker::PhantomData, rc::Rc};

#[doc(hidden)]
pub trait Worker: 'static {
    type Input: Message;
    type InitError: Message;
    type InitProgress: Message;
    type Client: Clone + 'static;
    fn __id() -> &'static str;
    fn __pool() -> bool;
    fn __client(service: Service) -> Self::Client;
}

#[derive(Clone)]
pub struct Service {
    endpoint: Rc<Endpoint>,
    instance: u64,
    owner: OwnerHandle,
    closing: Rc<RefCell<Option<js_sys::Promise>>>,
}
impl Service {
    pub fn job<T: Message, E: Message, P: Message>(
        &self,
        entry: &'static str,
        arguments: impl FnOnce(
            &crate::shared::Codec,
        ) -> Result<Vec<crate::shared::Payload>, WorkerError>
        + 'static,
    ) -> Job<T, E, P> {
        let state = State::new(
            &self.owner,
            entry,
            Requirement::Ordinary,
            Box::new(arguments),
            JobKind::Single,
        );
        {
            let mut state = state.borrow_mut();
            state.target = Target::Bound {
                endpoint: Rc::downgrade(&self.endpoint),
                instance: self.instance,
            };
        }
        Job {
            state,
            marker: PhantomData,
        }
    }
    pub fn close(&self) -> Close {
        let mut close = Close::new(Rc::clone(&self.endpoint), self.instance);
        close.closing = Rc::clone(&self.closing);
        close
    }
}
