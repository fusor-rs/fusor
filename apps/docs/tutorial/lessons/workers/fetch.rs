use fusor::OwnerHandle;
use fusor_worker::{JobError, TaskContext, TaskResult};

#[fusor_worker::task]
pub async fn fetch_text(path: String, ctx: TaskContext) -> TaskResult<String, String> {
    fusor_async::fetch::get_text(&path, &ctx.cancellation_token())
        .await
        .map_err(|error| JobError::Application(error.to_string()))
}

pub async fn run_fetch(owner: &OwnerHandle, path: String) -> TaskResult<String, String> {
    fetch_text::run(owner, path).await
}
