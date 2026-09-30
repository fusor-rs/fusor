use crate::{
    Message, Shared, WorkerError,
    shared::{Codec, Payload},
};
use fusor_async::{CancellationSource, CancellationToken};
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg_attr(
    not(all(target_arch = "wasm32", target_feature = "atomics")),
    allow(dead_code)
)]
pub(crate) struct ComputeBudget {
    pub queue: Mutex<ComputeQueue>,
    pub threads: usize,
    pub capacity: usize,
}

#[derive(Default)]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_feature = "atomics")),
    allow(dead_code)
)]
pub(crate) struct ComputeQueue {
    pub active: usize,
    pub waiting: std::collections::VecDeque<(bool, Box<dyn FnOnce() + Send>)>,
    pub bounded: usize,
}

pub(crate) struct Control {
    pub cancelled: AtomicBool,
    pub progress: Mutex<Option<Payload>>,
    pub codec: Codec,
    pub failure: Mutex<Option<WorkerError>>,
    #[cfg_attr(
        not(all(target_arch = "wasm32", target_feature = "atomics")),
        allow(dead_code)
    )]
    pub compute: Arc<ComputeBudget>,
}
impl Drop for Control {
    fn drop(&mut self) {
        if let Some(payload) = self.progress.get_mut().unwrap().take() {
            self.codec.discard(payload);
        }
    }
}

/// Local worker orchestration context. CPU work belongs in `compute`.
pub struct TaskContext<P = ()> {
    pub(crate) control: Arc<Control>,
    pub(crate) source: Rc<CancellationSource>,
    pub(crate) marker: PhantomData<fn(P)>,
}
/// Thread-safe cancellation and progress for synchronous computation.
pub struct ComputeContext<P = ()> {
    pub(crate) control: Arc<Control>,
    pub(crate) marker: PhantomData<fn(P)>,
}
impl<P: Message> Clone for ComputeContext<P> {
    fn clone(&self) -> Self {
        Self {
            control: Arc::clone(&self.control),
            marker: PhantomData,
        }
    }
}
impl<P: Message> ComputeContext<P> {
    pub fn report(&self, update: P) {
        if self.check_cancelled().is_err() {
            return;
        }
        match update.encode(crate::message::MESSAGE_LIMIT, &self.control.codec) {
            Ok(payload) => {
                let old = self.control.progress.lock().unwrap().replace(payload);
                if let Some(old) = old {
                    self.control.codec.discard(old);
                }
            }
            Err(error) => {
                *self.control.failure.lock().unwrap() = Some(error);
            }
        }
    }
    pub fn check_cancelled(&self) -> Result<(), WorkerError> {
        if self.control.cancelled.load(Ordering::Acquire) {
            Err(WorkerError::Cancelled)
        } else {
            Ok(())
        }
    }
    pub fn share<T: Send + Sync + 'static>(&self, value: T) -> Result<Shared<T>, WorkerError> {
        self.control.codec.share(value)
    }
    pub fn resolve<T: Send + Sync + 'static>(
        &self,
        value: &Shared<T>,
    ) -> Result<Arc<T>, WorkerError> {
        self.control.codec.resolve(value)
    }
}
impl<P: Message> TaskContext<P> {
    fn cpu(&self) -> ComputeContext<P> {
        ComputeContext {
            control: Arc::clone(&self.control),
            marker: PhantomData,
        }
    }
    pub fn report(&self, update: P) {
        self.cpu().report(update);
    }
    pub fn check_cancelled(&self) -> Result<(), WorkerError> {
        self.cpu().check_cancelled()
    }
    pub fn cancellation_token(&self) -> CancellationToken {
        self.source.token()
    }
    pub fn share<T: Send + Sync + 'static>(&self, value: T) -> Result<Shared<T>, WorkerError> {
        self.cpu().share(value)
    }
    pub fn resolve<T: Send + Sync + 'static>(
        &self,
        value: &Shared<T>,
    ) -> Result<Arc<T>, WorkerError> {
        self.cpu().resolve(value)
    }
    pub async fn compute<R, F>(&self, work: F) -> Result<R, WorkerError>
    where
        F: FnOnce(ComputeContext<P>) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.check_cancelled()?;
        crate::backend::compute(self.cpu(), work, true).await
    }
}
