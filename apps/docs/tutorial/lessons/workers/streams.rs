use fusor::OwnerHandle;
use fusor_worker::{StreamSender, TaskResult};

#[fusor_worker::task(stream)]
pub async fn batches(count: u32, mut output: StreamSender<Vec<u32>>) -> TaskResult<()> {
    let mut batch = Vec::with_capacity(32);
    for value in 0..count {
        batch.push(value);
        if batch.len() == 32 {
            output.send(std::mem::take(&mut batch)).await?;
        }
    }
    if !batch.is_empty() {
        output.send(batch).await?;
    }
    Ok(())
}

pub async fn collect_batches(owner: &OwnerHandle) -> TaskResult<Vec<u32>> {
    let mut output = batches::stream(owner, 100).buffer(2).max_batch_bytes(1024);
    let mut values = Vec::new();
    while let Some(batch) = output.next().await {
        values.extend(batch?);
    }
    Ok(values)
}
