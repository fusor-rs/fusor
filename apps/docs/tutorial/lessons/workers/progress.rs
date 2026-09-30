use fusor::OwnerHandle;
use fusor_async::CancellationToken;
use fusor_worker::{CancellationHandle, ComputeContext, TaskResult};

#[fusor_worker::task]
pub fn sum(count: u32, ctx: ComputeContext<u32>) -> TaskResult<u64> {
    let mut total = 0;
    for value in 0..count {
        ctx.check_cancelled()?;
        total += u64::from(value);
        ctx.report(value + 1);
    }
    Ok(total)
}

pub async fn run_with_progress(
    owner: &OwnerHandle,
    cancel: &CancellationToken,
    on_progress: impl FnMut(u32) + 'static,
    on_start: impl FnOnce(CancellationHandle),
) -> TaskResult<u64> {
    let job = sum::run(owner, 100)
        .cancel_on(cancel)
        .on_progress(on_progress);
    // Store this handle in UI state so a Cancel button can call cancel().
    on_start(job.cancellation_handle());
    job.await
}
