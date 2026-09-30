use fusor::OwnerHandle;
use fusor_worker::{Pool, TaskContext, TaskResult, WorkerError};

#[fusor_worker::task(pool)]
pub async fn summarize(values: Vec<u64>, ctx: TaskContext) -> TaskResult<u64> {
    let total = ctx
        .compute(move |cpu| -> Result<u64, WorkerError> {
            let mut total = 0;
            for value in values {
                cpu.check_cancelled()?;
                total += value;
            }
            Ok(total)
        })
        .await??;
    Ok(total)
}

pub async fn use_pool(owner: &OwnerHandle) -> TaskResult<u64> {
    let pool = Pool::new(owner)
        .max_threads(2)
        .max_async_jobs(4)
        .queue_capacity(64)
        .await?;
    let total = summarize::run(owner, vec![10, 20, 30]).on(&pool).await?;
    pool.close().await?;
    Ok(total)
}
