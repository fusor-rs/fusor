use super::{Service, Worker};
use crate::{
    Bound, Job, Message, Pool, TaskResult, Unbound, WorkerError,
    job::{Arguments, JobKind, State, Target},
};
use fusor::OwnerHandle;
use fusor_async::CancellationToken;
use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

pub fn spawn<W: Worker>(owner: &OwnerHandle, input: W::Input) -> Spawn<W> {
    let args: Arguments = Box::new(move |codec| Ok(vec![crate::__private::encode(&input, codec)?]));
    let state = State::new(owner, W::__id(), W::__pool().into(), args, JobKind::Single);
    state.borrow_mut().target = Target::Dedicated;
    Spawn {
        job: Job {
            state,
            marker: PhantomData,
        },
        marker: PhantomData,
    }
}
#[must_use]
pub struct Spawn<W: Worker, M = Unbound> {
    job: Job<InstanceId, W::InitError, W::InitProgress, M>,
    marker: PhantomData<fn() -> W>,
}
impl<W: Worker, M> Spawn<W, M> {
    pub fn on_progress(self, callback: impl FnMut(W::InitProgress) + 'static) -> Self {
        Self {
            job: self.job.on_progress(callback),
            marker: PhantomData,
        }
    }
    pub fn cancel_on(self, token: &CancellationToken) -> Self {
        Self {
            job: self.job.cancel_on(token),
            marker: PhantomData,
        }
    }
    pub fn cancellation_handle(&self) -> crate::CancellationHandle {
        self.job.cancellation_handle()
    }
    pub fn scope(self, owner: &OwnerHandle) -> Self {
        Self {
            job: self.job.scope(owner),
            marker: PhantomData,
        }
    }
}
impl<W: Worker> Spawn<W> {
    pub fn on(self, pool: &Pool) -> Spawn<W, Bound> {
        Spawn {
            job: self.job.on(pool),
            marker: PhantomData,
        }
    }
}
impl<W: Worker, M> Unpin for Spawn<W, M> {}
impl<W: Worker, M> Future for Spawn<W, M> {
    type Output = TaskResult<W::Client, W::InitError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let instance = match Pin::new(&mut self.job).poll(cx) {
            Poll::Ready(Ok(id)) => id.0,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Pending => return Poll::Pending,
        };
        let state = self.job.state.borrow();
        let Some(endpoint) = state.endpoint() else {
            return Poll::Ready(Err(WorkerError::Terminated.into()));
        };
        endpoint.own_service(instance, &state.owner);
        Poll::Ready(Ok(W::__client(Service {
            endpoint,
            instance,
            owner: state.owner.clone(),
            closing: Rc::default(),
        })))
    }
}

struct InstanceId(u64);
impl Message for InstanceId {}
impl crate::message::private::Sealed for InstanceId {
    fn encode(
        &self,
        _: usize,
        _: &crate::shared::Codec,
    ) -> Result<crate::shared::Payload, WorkerError> {
        Ok(crate::shared::Payload::Instance(self.0))
    }
    fn decode(
        payload: crate::shared::Payload,
        codec: &crate::shared::Codec,
    ) -> Result<Self, WorkerError> {
        if let crate::shared::Payload::Instance(id) = payload {
            Ok(Self(id))
        } else {
            codec.discard(payload);
            Err(WorkerError::IncompatibleArtifact)
        }
    }
}
