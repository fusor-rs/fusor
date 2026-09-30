use fusor::OwnerHandle;
use fusor_worker::TaskResult;

pub struct Counter(u64);

#[fusor_worker::worker]
impl Counter {
    pub fn new(initial: u64) -> TaskResult<Self> {
        Ok(Self(initial))
    }

    pub fn add(&mut self, amount: u64) -> TaskResult<u64> {
        self.0 += amount;
        Ok(self.0)
    }
}

pub async fn use_counter(owner: &OwnerHandle) -> TaskResult<u64> {
    let counter = fusor_worker::spawn::<Counter>(owner, 0).await?;
    let value = counter.add(3).await?;
    counter.close().await?;
    Ok(value)
}
