use fusor::Owner;
use fusor_async::CancellationSource;
use fusor_worker::{JobError, TaskResult, WorkerError, task, worker};
use futures_executor::block_on;
use std::cell::RefCell;

type Input = u32;
mod authoring {
    use super::{TaskResult, task, worker};
    pub type Output = super::Input;
    #[task]
    pub fn twice(input: super::Input) -> TaskResult<self::Output> {
        Ok(input * 2)
    }
    #[task(pool)]
    pub fn pool_only(input: ()) -> TaskResult<()> {
        Ok(input)
    }
    #[derive(serde::Serialize, serde::Deserialize)]
    pub struct Counter(pub u32);
    #[worker]
    impl Counter {
        pub fn new(input: super::Input) -> TaskResult<Self> {
            Ok(Self(input))
        }
        #[cfg(any())]
        pub fn new<T>(_: T) -> TaskResult<Self> {
            unreachable!()
        }
        #[cfg_attr(all(), cfg_attr(all(), cfg(any())))]
        pub fn close(&mut self) {}
        #[cfg_attr(all(), inline)]
        #[expect(
            clippy::needless_arbitrary_self_type,
            reason = "exercise explicit receiver syntax"
        )]
        pub fn add(self: &mut Self, input: super::Input) -> TaskResult<self::Output> {
            self.0 += input;
            Ok(self.0)
        }
        pub fn replace(&mut self, input: Self) -> TaskResult<Self> {
            Ok(std::mem::replace(self, input))
        }
    }
}
use authoring::{Counter, pool_only, twice};

#[test]
fn original_entry_points_stay_callable_without_a_runtime() {
    let _: <Counter as fusor_worker::Worker>::Input = 2_u32;
    assert_eq!(twice(21).unwrap(), 42);
    assert_eq!(Counter::new(2).unwrap().add(3).unwrap(), 5);
    assert_eq!(Counter::new(2).unwrap().replace(Counter(7)).unwrap().0, 2);
    fn send_sync<T: Send + Sync>() {}
    send_sync::<fusor_worker::ComputeContext<RefCell<String>>>();
}

#[test]
fn lazy_jobs_respect_cancellation_and_intersected_owner_lifetimes() {
    let owner = Owner::new();
    owner.commit();
    let token = CancellationSource::default();
    let job = twice::run(&owner.handle(), 2).cancel_on(&token.token());
    token.cancel();
    assert!(matches!(
        block_on(job),
        Err(JobError::Worker(WorkerError::Cancelled))
    ));
    let other = Owner::new();
    other.commit();
    let job = twice::run(&owner.handle(), 3).scope(&other.handle());
    other.dispose();
    assert!(matches!(
        block_on(job),
        Err(JobError::Worker(WorkerError::OwnerDisposed))
    ));
    let job = twice::run(&owner.handle(), 4);
    job.cancellation_handle().cancel();
    assert!(matches!(
        block_on(job),
        Err(JobError::Worker(WorkerError::Cancelled))
    ));
    assert!(matches!(
        block_on(pool_only::run(&owner.handle(), ())),
        Err(JobError::Worker(WorkerError::PoolRequired))
    ));
}

#[test]
fn native_jobs_never_fall_back_to_ui_execution() {
    let owner = Owner::new();
    owner.commit();
    assert!(matches!(
        block_on(twice::run(&owner.handle(), 21)),
        Err(JobError::Worker(WorkerError::Unsupported { .. }))
    ));
}
