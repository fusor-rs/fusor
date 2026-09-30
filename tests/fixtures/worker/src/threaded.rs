use super::*;
use fusor_worker::{Pool, Shared};

pub struct Dataset(Vec<u64>);
#[fusor_worker::task]
pub fn dataset(count: u32, ctx: ComputeContext) -> TaskResult<Shared<Dataset>> {
    Ok(ctx.share(Dataset((0..u64::from(count)).collect()))?)
}
#[fusor_worker::task]
pub fn sum(data: Shared<Dataset>, ctx: ComputeContext) -> TaskResult<u64> {
    use rayon::prelude::*;
    let data = ctx.resolve(&data)?;
    let parallel = data.0.par_iter().sum();
    assert_eq!(parallel, data.0.iter().sum::<u64>());
    Ok(parallel)
}
#[fusor_worker::task]
pub async fn compute(input: u32, ctx: TaskContext<u32>) -> TaskResult<u64> {
    Ok(ctx
        .compute(move |cpu| {
            cpu.report(input);
            u64::from(input) * 2
        })
        .await?)
}
#[wasm_bindgen]
pub async fn exercise_pool() {
    stage("Pool lifetime");
    let owner = fusor::Owner::new();
    owner.commit();
    let handle = owner.handle();
    let pool = Pool::new(&handle).max_threads(2).await.unwrap();
    assert!(pool.threads() > 0 && pool.threads() <= 2);
    exercise_documented_pool(&handle, &pool).await;
    check_leases(&handle, &pool).await;
    assert_eq!(
        execution_realm::run(&handle, ()).on(&pool).await.unwrap(),
        "fusor-compute"
    );
    let data = dataset::run(&handle, 1000).on(&pool).await.unwrap();
    assert_eq!(
        sum::run(&handle, data.clone()).on(&pool).await.unwrap(),
        499500
    );
    assert_eq!(compute::run(&handle, 21).on(&pool).await.unwrap(), 42);
    let second = Pool::new(&handle).max_threads(1).await.unwrap();
    assert!(matches!(
        sum::run(&handle, data.clone()).on(&second).await,
        Err(JobError::Worker(fusor_worker::WorkerError::WrongPool))
    ));
    let service = fusor_worker::spawn::<Counter>(&handle, 100)
        .on(&pool)
        .await
        .unwrap();
    assert_eq!(service.add(1).await.unwrap(), 101);
    service.close().await.unwrap();
    assert_eq!(twice::run(&handle, 5).on(&pool).await.unwrap(), 10);
    let slow = fusor_worker::spawn::<Delayed>(&handle, 0)
        .on(&pool)
        .await
        .unwrap();
    let mut pending = slow.change((1, 200));
    start(&mut pending).await;
    pause(20).await;
    assert!(matches!(
        slow.close()
            .timeout(std::time::Duration::from_millis(10))
            .await,
        Err(fusor_worker::WorkerError::CloseTimedOut)
    ));
    assert_eq!(twice::run(&handle, 7).on(&pool).await.unwrap(), 14);
    assert!(matches!(
        pending.await,
        Err(JobError::Worker(fusor_worker::WorkerError::CloseTimedOut))
    ));
    pool.close().await.unwrap();
    assert!(sum::run(&handle, data).on(&pool).await.is_err());
    second.terminate();
}

#[fusor_worker::task]
pub fn execution_realm(_: ()) -> TaskResult<String> {
    Ok(realm())
}

#[wasm_bindgen]
pub async fn pool_probe() -> String {
    let owner = fusor::Owner::new();
    owner.commit();
    match Pool::new(&owner.handle()).max_threads(2).await {
        Ok(pool) => {
            pool.terminate();
            "ready".into()
        }
        Err(error) => format!("{error:?}"),
    }
}
#[wasm_bindgen]
pub async fn exercise_compute() {
    stage("Compute queue");
    let owner = fusor::Owner::new();
    owner.commit();
    let handle = owner.handle();
    let pool = Pool::new(&handle)
        .max_threads(2)
        .max_async_jobs(1)
        .queue_capacity(1)
        .await
        .unwrap();
    if pool.threads() == 2 {
        let shared = overlap_state::run(&handle, ()).on(&pool).await.unwrap();
        let mut first = overlap::run(&handle, shared.clone()).on(&pool);
        start(&mut first).await;
        let second = overlap::run(&handle, shared).on(&pool).await.unwrap();
        assert!(first.await.unwrap() && second, "compute tasks must overlap");
    }
    let gate = compute_gate::run(&handle, ()).on(&pool).await.unwrap();
    let mut held = Vec::new();
    for _ in 0..pool.threads() {
        let mut job = hold_cpu::run(&handle, gate.clone()).on(&pool);
        start(&mut job).await;
        held.push(job);
    }
    assert!(compute_queue::run(&handle, gate).on(&pool).await.unwrap());
    for job in held {
        job.await.unwrap();
    }
    let mut jobs = Vec::new();
    for _ in 0..pool.threads() {
        let mut job = busy::run(&handle, 200).on(&pool);
        start(&mut job).await;
        jobs.push(job);
    }
    let mut queued = busy::run(&handle, 100).on(&pool);
    start(&mut queued).await;
    assert!(matches!(
        busy::run(&handle, 1).on(&pool).await,
        Err(JobError::Worker(fusor_worker::WorkerError::QueueFull {
            capacity: 1
        }))
    ));
    queued.cancellation_handle().cancel();
    stage("Compute and I/O overlap");
    // Coordinator I/O still runs while all CPU slots are occupied.
    assert_eq!(
        fetch_text::run(&handle, "sample.txt".into())
            .on(&pool)
            .await
            .unwrap(),
        "worker fetch from app base\n"
    );
    for job in jobs {
        assert!(job.await.unwrap() > 0);
    }
    stage("Compute crash");
    assert!(matches!(
        crash::run(&handle, ()).on(&pool).await,
        Err(JobError::Worker(fusor_worker::WorkerError::Crashed { .. }))
    ));
    assert!(twice::run(&handle, 1).on(&pool).await.is_err());
}

#[fusor_worker::task]
fn overlap_state(_: (), ctx: ComputeContext) -> TaskResult<Shared<std::sync::atomic::AtomicUsize>> {
    Ok(ctx.share(std::sync::atomic::AtomicUsize::new(0))?)
}
#[fusor_worker::task]
fn overlap(value: Shared<std::sync::atomic::AtomicUsize>, ctx: ComputeContext) -> TaskResult<bool> {
    use std::sync::atomic::Ordering;
    let value = ctx.resolve(&value)?;
    value.fetch_add(1, Ordering::AcqRel);
    let deadline = js_sys::Date::now() + 1000.0;
    while value.load(Ordering::Acquire) < 2 && js_sys::Date::now() < deadline {
        ctx.check_cancelled()?;
    }
    Ok(value.load(Ordering::Acquire) == 2)
}

#[fusor_worker::task]
fn compute_gate(_: (), ctx: ComputeContext) -> TaskResult<Shared<std::sync::atomic::AtomicBool>> {
    Ok(ctx.share(std::sync::atomic::AtomicBool::new(false))?)
}
#[fusor_worker::task]
fn hold_cpu(gate: Shared<std::sync::atomic::AtomicBool>, ctx: ComputeContext) -> TaskResult<()> {
    let gate = ctx.resolve(&gate)?;
    while !gate.load(std::sync::atomic::Ordering::Acquire) {
        ctx.check_cancelled()?;
    }
    Ok(())
}
#[fusor_worker::task]
async fn compute_queue(
    gate: Shared<std::sync::atomic::AtomicBool>,
    ctx: TaskContext,
) -> TaskResult<bool> {
    use std::{future::Future, sync::atomic::Ordering, task::Poll};
    let mut first = Box::pin(ctx.compute(|_| 21));
    start(&mut first).await;
    let mut second = Box::pin(ctx.compute(|_| 42));
    let overflow = std::future::poll_fn(|cx| Poll::Ready(second.as_mut().poll(cx))).await;
    ctx.resolve(&gate)?.store(true, Ordering::Release);
    assert_eq!(first.await?, 21);
    Ok(matches!(
        overflow,
        Poll::Ready(Err(fusor_worker::WorkerError::QueueFull { capacity: 1 }))
    ))
}

static DROPPED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub struct LifetimeProbe;
impl Drop for LifetimeProbe {
    fn drop(&mut self) {
        DROPPED.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
}
#[fusor_worker::worker]
impl LifetimeProbe {
    pub async fn new(milliseconds: u32) -> TaskResult<Self> {
        pause(milliseconds).await;
        Ok(Self)
    }
}
#[fusor_worker::task]
async fn lease_probe(milliseconds: u32, ctx: TaskContext) -> TaskResult<Shared<LifetimeProbe>> {
    let data = ctx.share(LifetimeProbe)?;
    pause(milliseconds).await;
    Ok(data)
}
#[fusor_worker::task]
async fn drop_count(_: ()) -> TaskResult<u32> {
    Ok(DROPPED.load(std::sync::atomic::Ordering::Acquire))
}

async fn check_leases(owner: &fusor::OwnerHandle, pool: &Pool) {
    let initial = drop_count::run(owner, ()).on(pool).await.unwrap();
    let lease = lease_probe::run(owner, 0).on(pool).await.unwrap();
    let clone = lease.clone();
    drop(lease);
    assert_eq!(drop_count::run(owner, ()).on(pool).await.unwrap(), initial);
    drop(clone);
    assert_eq!(
        drop_count::run(owner, ()).on(pool).await.unwrap(),
        initial + 1
    );
    let mut cancelled = lease_probe::run(owner, 60).on(pool);
    start(&mut cancelled).await;
    pause(20).await;
    cancelled.cancellation_handle().cancel();
    assert!(matches!(
        cancelled.await,
        Err(JobError::Worker(fusor_worker::WorkerError::Cancelled))
    ));
    let mut constructor = fusor_worker::spawn::<LifetimeProbe>(owner, 60).on(pool);
    start(&mut constructor).await;
    pause(20).await;
    constructor.cancellation_handle().cancel();
    assert!(matches!(
        constructor.await,
        Err(JobError::Worker(fusor_worker::WorkerError::Cancelled))
    ));
    // Wait for both deliberately uncooperative remote futures to finish.
    for _ in 0..100 {
        if drop_count::run(owner, ()).on(pool).await.unwrap() == initial + 3 {
            return;
        }
        pause(5).await;
    }
    panic!("cancelled replies and late initialization must release state");
}
