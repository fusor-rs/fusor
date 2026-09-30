use fusor::OwnerHandle;
use fusor_worker::TaskResult;

#[fusor_worker::task]
pub fn total(values: Vec<u64>) -> TaskResult<u64> {
    Ok(values.into_iter().sum())
}

pub async fn run_total(owner: &OwnerHandle) -> TaskResult<u64> {
    total::run(owner, vec![10, 20, 30]).await
}
