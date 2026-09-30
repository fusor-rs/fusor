mod state;
pub(crate) use state::{Arguments, Event, State};

use crate::{
    JobError, Message, NoError, Pool, TaskResult, WorkerError,
    shared::{Codec, Payload},
};
use fusor::OwnerHandle;
use fusor_async::CancellationToken;
use std::{
    cell::RefCell,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

pub struct Unbound;
pub struct Bound;

#[derive(Clone)]
pub struct CancellationHandle(pub(crate) Rc<dyn Fn()>);
impl CancellationHandle {
    pub fn cancel(&self) {
        (self.0)();
    }
}

type JobTypes<T, E, P, M> = PhantomData<fn() -> (T, E, P, M)>;
#[must_use]
pub struct Job<T, E = NoError, P = (), M = Bound> {
    pub(crate) state: Rc<RefCell<State>>,
    pub(crate) marker: JobTypes<T, E, P, M>,
}
impl<T, E, P, M> Unpin for Job<T, E, P, M> {}
impl<T: Message, E: Message, P: Message, M> Job<T, E, P, M> {
    pub fn on_progress(self, callback: impl FnMut(P) + 'static) -> Self {
        if State::configure(&self.state) {
            State::progress(&self.state, callback);
        }
        self
    }
    pub fn cancel_on(self, token: &CancellationToken) -> Self {
        if State::configure(&self.state) {
            State::token(&self.state, token);
        }
        self
    }
    pub fn cancellation_handle(&self) -> CancellationHandle {
        let weak = Rc::downgrade(&self.state);
        CancellationHandle(Rc::new(move || {
            if let Some(state) = weak.upgrade() {
                State::abort(&state, WorkerError::Cancelled);
            }
        }))
    }
    pub fn scope(self, owner: &OwnerHandle) -> Self {
        if State::configure(&self.state) {
            State::scope(&self.state, owner);
        }
        self
    }
}
impl<T: Message, E: Message, P: Message> Job<T, E, P, Unbound> {
    pub fn on(self, pool: &Pool) -> Job<T, E, P, Bound> {
        if State::configure(&self.state) {
            pool.bind(&self.state);
        }
        Job {
            state: self.state,
            marker: PhantomData,
        }
    }
}
impl<T: Message, E: Message, P: Message, M> Future for Job<T, E, P, M> {
    type Output = TaskResult<T, E>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        State::start(&self.state);
        let mut state = self.state.borrow_mut();
        if let Some(error) = state.abort.take() {
            state.committed = true;
            return Poll::Ready(Err(error.into()));
        }
        if state.committed {
            panic!("Job polled after completion");
        }
        if let Some(Event::Result(result)) = state.events.pop_front() {
            state.committed = true;
            let codec = state
                .endpoint
                .upgrade()
                .map(|e| e.codec.clone())
                .unwrap_or_default();
            drop(state);
            return Poll::Ready(decode_result(result, &codec));
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}
pub(crate) fn decode_result<T: Message, E: Message>(
    result: Result<Payload, JobError<Payload>>,
    codec: &Codec,
) -> TaskResult<T, E> {
    match result {
        Ok(payload) => Ok(T::decode(payload, codec)?),
        Err(JobError::Application(payload)) => {
            Err(JobError::Application(E::decode(payload, codec)?))
        }
        Err(JobError::Worker(error)) => Err(error.into()),
    }
}
