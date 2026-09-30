use fusor::OwnerHandle;
use fusor_worker::{ComputeContext, Pool, Shared, TaskResult};

#[fusor_worker::task(pool)]
pub fn dataset(count: u64, ctx: ComputeContext) -> TaskResult<Shared<Vec<u64>>> {
    Ok(ctx.share((0..count).collect::<Vec<_>>())?)
}

#[fusor_worker::task(pool)]
pub fn summarize(data: Shared<Vec<u64>>, ctx: ComputeContext) -> TaskResult<u64> {
    Ok(ctx.resolve(&data)?.iter().sum())
}

pub async fn use_shared(owner: &OwnerHandle, pool: &Pool) -> TaskResult<u64> {
    let data = dataset::run(owner, 100).on(pool).await?;
    let first = summarize::run(owner, data.clone()).on(pool).await?;
    let second = summarize::run(owner, data).on(pool).await?;
    Ok(first + second)
}
