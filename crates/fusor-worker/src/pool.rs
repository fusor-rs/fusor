use crate::{
    CancellationHandle, Close, WorkerError,
    endpoint::{DEFAULT_QUEUE_CAPACITY, Endpoint, RuntimeMode, capabilities},
    job::{Requirement, State, Target},
};
use fusor::OwnerHandle;
use fusor_async::{CancelRegistration, CancellationSource, CancellationToken};
use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

pub struct Pool {
    pub(crate) endpoint: Rc<Endpoint>,
    pub(crate) owner: OwnerHandle,
    threads: usize,
}
impl Pool {
    #[expect(
        clippy::new_ret_no_self,
        reason = "pool creation returns an initialization future"
    )]
    pub fn new(owner: &OwnerHandle) -> PoolInit {
        #[cfg(target_arch = "wasm32")]
        crate::bridge::require_pool();
        PoolInit {
            owner: owner.clone(),
            cancellation: Rc::new(CancellationSource::default()),
            tokens: Vec::new(),
            threads: capabilities().hardware_parallelism.min(4),
            active: 4,
            capacity: DEFAULT_QUEUE_CAPACITY,
            future: None,
        }
    }
    pub fn threads(&self) -> usize {
        self.threads
    }
    pub fn close(&self) -> Close {
        Close::new(Rc::clone(&self.endpoint), 0)
    }
    pub fn terminate(&self) {
        self.endpoint.terminate(WorkerError::Terminated);
    }
    pub(crate) fn bind(&self, state: &Rc<RefCell<State>>) {
        #[cfg(target_arch = "wasm32")]
        crate::bridge::require_pool();
        {
            let mut state = state.borrow_mut();
            state.target = Target::Bound {
                endpoint: Rc::downgrade(&self.endpoint),
                instance: 0,
            };
            state.requirement = Requirement::Pool;
        }
        State::scope(state, &self.owner);
    }
}
impl Drop for Pool {
    fn drop(&mut self) {
        self.terminate();
    }
}

type Initialization = Pin<Box<dyn Future<Output = Result<Pool, WorkerError>>>>;
#[must_use]
pub struct PoolInit {
    owner: OwnerHandle,
    cancellation: Rc<CancellationSource>,
    tokens: Vec<(CancellationToken, CancelRegistration)>,
    threads: usize,
    active: usize,
    capacity: usize,
    future: Option<Initialization>,
}
impl PoolInit {
    fn configure(&mut self) -> bool {
        if self.future.is_some() {
            self.future = Some(Box::pin(async {
                Err(crate::error::configuration(
                    "builders must be configured before their first poll",
                ))
            }));
            false
        } else {
            true
        }
    }
    pub fn max_threads(mut self, count: usize) -> Self {
        if self.configure() {
            self.threads = count;
        }
        self
    }
    pub fn max_async_jobs(mut self, count: usize) -> Self {
        if self.configure() {
            self.active = count;
        }
        self
    }
    pub fn queue_capacity(mut self, count: usize) -> Self {
        if self.configure() {
            self.capacity = count;
        }
        self
    }
    pub fn cancel_on(mut self, token: &CancellationToken) -> Self {
        if self.configure() {
            let cancel = self.cancellation_handle();
            self.tokens
                .push((token.clone(), token.on_cancel(move || cancel.cancel())));
        }
        self
    }
    pub fn cancellation_handle(&self) -> CancellationHandle {
        let weak = Rc::downgrade(&self.cancellation);
        CancellationHandle(Rc::new(move || {
            if let Some(source) = weak.upgrade() {
                source.cancel();
            }
        }))
    }
}
impl Future for PoolInit {
    type Output = Result<Pool, WorkerError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.future.is_none() {
            if self.owner.is_disposed() {
                return Poll::Ready(Err(WorkerError::OwnerDisposed));
            }
            let token = self.cancellation.token();
            if token.is_cancelled() {
                return Poll::Ready(Err(WorkerError::Cancelled));
            }
            if self.threads == 0 || self.active == 0 {
                return Poll::Ready(Err(crate::error::configuration(
                    "thread and async limits must be nonzero",
                )));
            }
            let threads = self.threads.min(capabilities().hardware_parallelism);
            let endpoint = match Endpoint::create(
                &self.owner,
                RuntimeMode::Pool {
                    threads,
                    active: self.active,
                    capacity: self.capacity,
                },
            ) {
                Ok(endpoint) => endpoint,
                Err(error) => return Poll::Ready(Err(error)),
            };
            let pool = Pool {
                endpoint,
                owner: self.owner.clone(),
                threads,
            };
            self.future = Some(Box::pin(async move {
                let weak = Rc::downgrade(&pool.endpoint);
                let _cancel = token.on_cancel(move || {
                    if let Some(endpoint) = weak.upgrade() {
                        endpoint.terminate(WorkerError::Cancelled);
                    }
                });
                pool.endpoint.ready().await?;
                Ok(pool)
            }));
        }
        self.future
            .as_mut()
            .expect("initialization was installed before polling")
            .as_mut()
            .poll(cx)
    }
}
