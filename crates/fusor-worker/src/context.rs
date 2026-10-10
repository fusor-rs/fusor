use crate::{
    Message, Shared, WorkerError,
    shared::{Codec, Payload},
};
use fusor_async::{CancellationSource, CancellationToken};
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg_attr(
    not(all(target_arch = "wasm32", target_feature = "atomics")),
    expect(
        dead_code,
        reason = "compute scheduling runs only in shared-memory Wasm workers"
    )
)]
pub(crate) struct ComputeBudget {
    pub queue: Mutex<ComputeQueue>,
    pub threads: usize,
    pub capacity: usize,
}

#[derive(Default)]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_feature = "atomics")),
    expect(
        dead_code,
        reason = "compute scheduling runs only in shared-memory Wasm workers"
    )
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
        expect(
            dead_code,
            reason = "compute scheduling runs only in shared-memory Wasm workers"
        )
    )]
    pub compute: Arc<ComputeBudget>,
}
impl Drop for Control {
    fn drop(&mut self) {
        // A slot always owns a complete payload, including after a poisoned write.
        if let Some(payload) = self
            .progress
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            self.codec.discard(payload);
        }
    }
}

impl Control {
    fn report<P: Message>(&self, update: P) {
        if self.check_cancelled().is_err() {
            return;
        }
        match update.encode(crate::message::MESSAGE_LIMIT, &self.codec) {
            Ok(payload) => {
                let old = self.progress().replace(payload);
                if let Some(old) = old {
                    self.codec.discard(old);
                }
            }
            Err(error) => self.fail(error),
        }
    }

    fn check_cancelled(&self) -> Result<(), WorkerError> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(WorkerError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub(crate) fn progress(&self) -> MutexGuard<'_, Option<Payload>> {
        self.progress.lock().unwrap_or_else(|error| {
            self.fail(WorkerError::Crashed {
                message: "progress slot was poisoned".into(),
            });
            // Recover ownership so pending transfers can still be discarded.
            error.into_inner()
        })
    }

    pub(crate) fn fail(&self, error: WorkerError) {
        // Replacing a complete error does not depend on the previous slot contents.
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
    }

    pub(crate) fn take_failure(&self) -> Option<WorkerError> {
        match self.failure.lock() {
            Ok(mut failure) => failure.take(),
            Err(_) => Some(WorkerError::Crashed {
                message: "failure slot was poisoned".into(),
            }),
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
        self.control.report(update);
    }
    pub fn check_cancelled(&self) -> Result<(), WorkerError> {
        self.control.check_cancelled()
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
        self.control.report(update);
    }
    pub fn check_cancelled(&self) -> Result<(), WorkerError> {
        self.control.check_cancelled()
    }
    pub fn cancellation_token(&self) -> CancellationToken {
        self.source.token()
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
    pub async fn compute<R, F>(&self, work: F) -> Result<R, WorkerError>
    where
        F: FnOnce(ComputeContext<P>) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.check_cancelled()?;
        crate::backend::compute(self.cpu(), work, true).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::private::Sealed;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn poisoned_progress_still_releases_transferred_allocations() {
        for report in [false, true] {
            let value = Arc::new(());
            let codec = Codec::local("pool".into(), "generation".into());
            let shared = codec.share(Arc::clone(&value)).unwrap();
            let payload = shared.encode(1024, &codec).unwrap();
            drop(shared);
            let control = Arc::new(Control {
                cancelled: AtomicBool::new(false),
                progress: Mutex::new(Some(payload)),
                codec,
                failure: Mutex::new(None),
                compute: Arc::new(ComputeBudget {
                    queue: Mutex::default(),
                    threads: 1,
                    capacity: 1,
                }),
            });
            assert!(
                catch_unwind(AssertUnwindSafe(|| {
                    let _guard = control.progress.lock().unwrap();
                    panic!("interrupted progress write");
                }))
                .is_err()
            );
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                if report {
                    ComputeContext::<u32> {
                        control: Arc::clone(&control),
                        marker: PhantomData,
                    }
                    .report(7);
                    assert!(matches!(
                        control.take_failure(),
                        Some(WorkerError::Crashed { .. })
                    ));
                }
                drop(control);
            }));
            assert!(
                outcome.is_ok(),
                "reporting and cleanup must survive a poisoned progress slot"
            );
            assert_eq!(
                Arc::strong_count(&value),
                1,
                "the pending transfer was released"
            );
        }
    }
}
