use crate::{
    Bound, CancellationHandle, Job, JobError, Message, NoError, Pool, TaskResult, Unbound,
    WorkerError,
    context::Control,
    job::{Completion, Event, State, decode_result},
};
use fusor::OwnerHandle;
use fusor_async::CancellationToken;
use futures_core::Stream;
use std::{
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    sync::{Arc, atomic::Ordering},
    task::{Context, Poll},
};

/// A producer's bounded output. Capacity includes items already in transit.
pub struct StreamSender<T> {
    pub(crate) id: u64,
    pub(crate) limit: usize,
    pub(crate) control: Arc<Control>,
    pub(crate) marker: PhantomData<(T, Rc<()>)>,
}
struct Reservation(u64);
impl Drop for Reservation {
    fn drop(&mut self) {
        crate::bridge::return_credit(self.0);
    }
}
impl<T: Message> StreamSender<T> {
    pub async fn send(&mut self, item: T) -> Result<(), WorkerError> {
        if self.control.cancelled.load(Ordering::Acquire) {
            return Err(WorkerError::Cancelled);
        }
        let _reservation = Reservation(self.id);
        wasm_bindgen_futures::JsFuture::from(crate::bridge::reserve(self.id))
            .await
            .map_err(|_| WorkerError::Cancelled)?;
        if self.control.cancelled.load(Ordering::Acquire) {
            return Err(WorkerError::Cancelled);
        }
        let payload = item.encode(self.limit, &self.control.codec)?;
        let event = Event::Item(payload);
        let encoded = match crate::message::encode_json(&event, crate::message::MESSAGE_LIMIT) {
            Ok(encoded) => encoded,
            Err(error) => {
                if let Event::Item(payload) = event {
                    self.control.codec.discard(payload);
                }
                return Err(error);
            }
        };
        crate::bridge::emit(self.id, &encoded);
        Ok(())
    }
}

#[must_use]
pub struct ResultStream<T, E = NoError, P = (), M = Unbound> {
    pub(crate) job: Job<T, E, P, M>,
}
impl<T: Message, E: Message, P: Message, M> ResultStream<T, E, P, M> {
    pub async fn next(&mut self) -> Option<TaskResult<T, E>> {
        std::future::poll_fn(|cx| Pin::new(&mut *self).poll_next(cx)).await
    }
    pub fn buffer(self, batches: usize) -> Self {
        if State::configure(&self.job.state) {
            if batches == 0 {
                self.invalid_configuration("stream buffer must be nonzero");
            } else {
                self.job.state.borrow_mut().capacity = batches;
            }
        }
        self
    }
    pub fn max_batch_bytes(self, bytes: usize) -> Self {
        if State::configure(&self.job.state) {
            if bytes == 0 || bytes > crate::message::MESSAGE_LIMIT {
                self.invalid_configuration("stream batch limit must be between 1 and 16 MiB");
            } else {
                self.job.state.borrow_mut().batch_bytes = bytes;
            }
        }
        self
    }
    pub fn on_progress(self, callback: impl FnMut(P) + 'static) -> Self {
        Self {
            job: self.job.on_progress(callback),
        }
    }
    pub fn cancel_on(self, token: &CancellationToken) -> Self {
        Self {
            job: self.job.cancel_on(token),
        }
    }
    pub fn cancellation_handle(&self) -> CancellationHandle {
        self.job.cancellation_handle()
    }
    pub fn scope(self, owner: &OwnerHandle) -> Self {
        Self {
            job: self.job.scope(owner),
        }
    }
    fn invalid_configuration(&self, message: &str) {
        State::abort(&self.job.state, crate::error::configuration(message));
    }
}
impl<T: Message, E: Message, P: Message> ResultStream<T, E, P, Unbound> {
    pub fn on(self, pool: &Pool) -> ResultStream<T, E, P, Bound> {
        ResultStream {
            job: self.job.on(pool),
        }
    }
}
impl<T: Message, E: Message, P: Message, M> Stream for ResultStream<T, E, P, M> {
    type Item = TaskResult<T, E>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let raw = &self.job.state;
        State::start(raw);
        let mut state = raw.borrow_mut();
        if matches!(state.completion, Completion::Consumed) {
            return Poll::Ready(None);
        }
        if let Some(error) = state.completion.take_error() {
            return Poll::Ready(Some(Err(error.into())));
        }
        let endpoint = state.endpoint();
        let codec = endpoint
            .as_ref()
            .map(|e| e.codec.clone())
            .unwrap_or_default();
        match state.events.pop_front() {
            Some(Event::Item(payload)) => {
                let id = state
                    .admission
                    .as_ref()
                    .expect("stream events arrive only after job admission")
                    .id;
                drop(state);
                let result = decode_result(Ok(payload), &codec);
                if let Err(JobError::Worker(error)) = &result {
                    State::abort(raw, error.clone());
                    let mut state = raw.borrow_mut();
                    state.completion = Completion::Consumed;
                }
                if let Some(endpoint) = endpoint {
                    endpoint.credit(id);
                }
                Poll::Ready(Some(result))
            }
            Some(Event::End(result)) => {
                state.completion = Completion::Consumed;
                if let Some(endpoint) = endpoint {
                    endpoint.forget(
                        state
                            .admission
                            .as_ref()
                            .expect("stream events arrive only after job admission")
                            .id,
                    );
                }
                drop(state);
                Poll::Ready(match result {
                    Ok(()) => None,
                    Err(error) => Some(decode_result(Err(error), &codec)),
                })
            }
            Some(Event::Result(Err(error))) => {
                state.completion = Completion::Consumed;
                drop(state);
                Poll::Ready(Some(decode_result(Err(error), &codec)))
            }
            _ => {
                state.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}
