#![forbid(unsafe_code)]
#![cfg_attr(fusor_worker, allow(dead_code))]
#[cfg(all(target_arch = "wasm32", not(worker_config)))]
compile_error!("worker builds must preserve Cargo target rustflags");
use fusor_worker::{ComputeContext, JobError, StreamSender, TaskContext, TaskResult};
use wasm_bindgen::prelude::*;

#[path = "../../../../apps/docs/tutorial/lessons/workers/errors.rs"]
mod docs_errors;
#[path = "../../../../apps/docs/tutorial/lessons/workers/fetch.rs"]
mod docs_fetch;
#[cfg(feature = "pool")]
#[path = "../../../../apps/docs/tutorial/lessons/workers/pools.rs"]
mod docs_pools;
#[path = "../../../../apps/docs/tutorial/lessons/workers/progress.rs"]
mod docs_progress;
#[path = "../../../../apps/docs/tutorial/lessons/workers/services.rs"]
mod docs_services;
#[cfg(feature = "pool")]
#[path = "../../../../apps/docs/tutorial/lessons/workers/shared.rs"]
mod docs_shared;
#[path = "../../../../apps/docs/tutorial/lessons/workers/streams.rs"]
mod docs_streams;
#[path = "../../../../apps/docs/tutorial/lessons/workers/tasks.rs"]
mod docs_tasks;

struct App;
#[derive(fusor::FromInputs)]
struct UiOnly;
fusor::template!("web/index.html");

#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum DomainError {
    Negative,
}
#[fusor_worker::task]
pub fn twice(input: i32) -> TaskResult<i32, DomainError> {
    if input < 0 {
        return Err(JobError::Application(DomainError::Negative));
    }
    Ok(input * 2)
}
mod private {
    use super::*;
    use fusor_worker::task as background;
    #[background]
    pub fn twice(input: i32, ctx: ComputeContext<u32>) -> TaskResult<i32> {
        ctx.report(1);
        ctx.check_cancelled()?;
        Ok(input * 3)
    }
}
#[fusor_worker::task(stream)]
pub async fn numbers(
    count: u32,
    ctx: TaskContext<u32>,
    mut output: StreamSender<u32>,
) -> TaskResult<()> {
    for number in 0..count {
        ctx.check_cancelled()?;
        output.send(number).await?;
        ctx.report(number);
    }
    Ok(())
}
pub struct Counter(u32);
#[fusor_worker::worker]
impl Counter {
    const LEN: usize = 2;
    pub fn new(value: u32) -> TaskResult<Self> {
        Ok(Self(value))
    }
    #[cfg(any())]
    pub fn disabled<T>(&mut self, value: T) -> TaskResult<T> {
        Ok(value)
    }
    /// Add to the counter and return its new value.
    pub async fn add(&mut self, value: u32) -> TaskResult<u32> {
        self.0 += value;
        Ok(self.0)
    }
    pub fn terminate(&mut self, value: [u8; Self::LEN]) -> TaskResult<[u8; Self::LEN]> {
        Ok(value)
    }
}
#[wasm_bindgen]
pub async fn exercise() {
    std::panic::set_hook(Box::new(|info| panic_message(&info.to_string())));
    let owner = fusor::Owner::new();
    owner.commit();
    let handle = owner.handle();
    exercise_documented_tasks(&handle).await;
    let value = twice::run(&handle, 21).await.unwrap();
    assert_eq!(value, 42);
    assert_eq!(worker_tasks::twice::run(&handle, 3).await.unwrap(), 12);
    assert!(matches!(
        twice::run(&handle, -1).await,
        Err(JobError::Application(DomainError::Negative))
    ));
    assert_eq!(private::twice::run(&handle, 7).await.unwrap(), 21);
    let first = fusor_worker::spawn::<worker_tasks::first::Client>(&handle, 20)
        .await
        .unwrap();
    let second = fusor_worker::spawn::<worker_tasks::second::Client>(&handle, 30)
        .await
        .unwrap();
    assert_eq!(first.add(1).await.unwrap(), 21);
    assert_eq!(second.add(1).await.unwrap(), 31);
    let client = fusor_worker::spawn::<Counter>(&handle, 10).await.unwrap();
    assert_eq!(client.terminate([1, 2]).await.unwrap(), [1, 2]);
    assert_eq!(client.add(2).await.unwrap(), 12);
    assert_eq!(client.clone().add(3).await.unwrap(), 15);
    let other = fusor_worker::spawn::<Counter>(&handle, 100).await.unwrap();
    assert_eq!(other.add(1).await.unwrap(), 101);
    assert_eq!(client.add(1).await.unwrap(), 16);
    let mut stream = numbers::stream(&handle, 20).buffer(2);
    let mut values = Vec::new();
    while let Some(value) = stream.next().await {
        values.push(value.unwrap());
    }
    assert_eq!(values, (0..20).collect::<Vec<_>>());
    client.close().await.unwrap();
    let token = fusor_async::CancellationSource::default();
    token.cancel();
    assert!(matches!(
        twice::run(&handle, 4).cancel_on(&token.token()).await,
        Err(JobError::Worker(fusor_worker::WorkerError::Cancelled))
    ));
    let job = twice::run(&handle, 3);
    owner.dispose();
    assert!(matches!(
        job.await,
        Err(JobError::Worker(fusor_worker::WorkerError::OwnerDisposed))
    ));
}
#[cfg(test)]
mod tests {
    #[test]
    fn original_functions_remain_native() {
        assert_eq!(super::twice(21).unwrap(), 42);
    }

    #[test]
    fn documented_task_remains_native() {
        assert_eq!(super::docs_tasks::total(vec![10, 20, 30]).unwrap(), 60);
    }

    #[test]
    fn documented_service_remains_native() {
        let mut counter = super::docs_services::Counter::new(10).unwrap();
        assert_eq!(counter.add(3).unwrap(), 13);
    }
}

async fn exercise_documented_tasks(owner: &fusor::OwnerHandle) {
    assert_eq!(docs_tasks::run_total(owner).await.unwrap(), 60);
    assert_eq!(
        docs_errors::parse_count::run(owner, "42".into())
            .await
            .unwrap(),
        42
    );
    assert!(matches!(
        docs_errors::parse_count::run(owner, "invalid".into()).await,
        Err(JobError::Application(message)) if !message.is_empty()
    ));
    assert_eq!(docs_services::use_counter(owner).await.unwrap(), 3);
    assert_eq!(
        docs_streams::collect_batches(owner).await.unwrap(),
        (0..100).collect::<Vec<_>>()
    );
    assert_eq!(
        docs_fetch::run_fetch(owner, "sample.txt".into())
            .await
            .unwrap(),
        "worker fetch from app base\n"
    );
    let cancel = fusor_async::CancellationSource::default();
    let last_progress = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed = std::rc::Rc::clone(&last_progress);
    assert_eq!(
        docs_progress::run_with_progress(
            owner,
            &cancel.token(),
            move |value| observed.set(value),
            drop,
        )
        .await
        .unwrap(),
        4950
    );
    assert_eq!(last_progress.get(), 100);
    assert!(matches!(
        docs_progress::run_with_progress(owner, &cancel.token(), |_| {}, |handle| handle.cancel())
            .await,
        Err(JobError::Worker(fusor_worker::WorkerError::Cancelled))
    ));
}

#[cfg(feature = "pool")]
async fn exercise_documented_pool(owner: &fusor::OwnerHandle, pool: &fusor_worker::Pool) {
    assert_eq!(docs_pools::use_pool(owner).await.unwrap(), 60);
    assert_eq!(docs_shared::use_shared(owner, pool).await.unwrap(), 9900);
}

#[cfg(feature = "pool")]
mod threaded;

#[wasm_bindgen(
    inline_js = "export function delay(ms) { return new Promise(resolve => setTimeout(resolve, ms)); } export function realm() { return globalThis.name; } export function stage(name) { globalThis.workerStage = name; }"
)]
extern "C" {
    fn delay(ms: u32) -> js_sys::Promise;
    fn realm() -> String;
    fn stage(name: &str);
}
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn panic_message(message: &str);
}
async fn pause(ms: u32) {
    wasm_bindgen_futures::JsFuture::from(delay(ms))
        .await
        .unwrap();
}
#[fusor_worker::task]
pub async fn progress(_: (), ctx: TaskContext<u32>) -> TaskResult<()> {
    for value in 0..4 {
        ctx.report(value);
        pause(25).await;
        ctx.check_cancelled()?;
    }
    Ok(())
}
#[fusor_worker::task(stream)]
pub async fn failing_stream(_: (), mut output: StreamSender<u32>) -> TaskResult<(), DomainError> {
    for value in 0..3 {
        // Dropping a send must return both granted and still-pending credit.
        let mut abandoned = Box::pin(output.send(99));
        start(&mut abandoned).await;
        drop(abandoned);
        output.send(value).await?;
    }
    Err(JobError::Application(DomainError::Negative))
}
#[fusor_worker::worker]
impl Delayed {
    pub fn new(value: u32) -> TaskResult<Self> {
        Ok(Self(value))
    }
    pub async fn change(&mut self, input: (u32, u32)) -> TaskResult<u32> {
        self.0 += input.0;
        pause(input.1).await;
        self.0 += input.0;
        Ok(self.0)
    }
    pub fn read(&mut self, _: ()) -> TaskResult<u32> {
        Ok(self.0)
    }
}
pub struct Delayed(u32);
async fn start<F: std::future::Future + Unpin>(future: &mut F) {
    use std::{pin::Pin, task::Poll};
    std::future::poll_fn(|cx| {
        let _ = Pin::new(&mut *future).poll(cx);
        Poll::Ready(())
    })
    .await;
}
#[wasm_bindgen]
pub async fn exercise_lifetimes() {
    let owner = std::rc::Rc::new(fusor::Owner::new());
    owner.commit();
    let handle = owner.handle();
    let service = fusor_worker::spawn::<Delayed>(&handle, 0).await.unwrap();
    let mut first = service.change((10, 80));
    start(&mut first).await;
    pause(20).await;
    first.cancellation_handle().cancel();
    assert!(matches!(
        first.await,
        Err(JobError::Worker(fusor_worker::WorkerError::Cancelled))
    ));
    assert_eq!(service.read(()).await.unwrap(), 20);
    let mut stream = failing_stream::stream(&handle, ()).buffer(1);
    for expected in 0..3 {
        assert_eq!(stream.next().await.unwrap().unwrap(), expected);
    }
    assert!(matches!(
        stream.next().await,
        Some(Err(JobError::Application(DomainError::Negative)))
    ));
    assert!(stream.next().await.is_none());
    let mut blocked = numbers::stream(&handle, 100).buffer(1);
    assert_eq!(blocked.next().await.unwrap().unwrap(), 0);
    drop(blocked);
    assert_eq!(twice::run(&handle, 4).await.unwrap(), 8);
    let mut bounded = numbers::stream(&handle, 11).max_batch_bytes(1);
    for expected in 0..10 {
        assert_eq!(bounded.next().await.unwrap().unwrap(), expected);
    }
    assert!(matches!(
        bounded.next().await,
        Some(Err(JobError::Worker(
            fusor_worker::WorkerError::PayloadTooLarge { .. }
        )))
    ));
    let scopes = std::rc::Rc::clone(&owner);
    assert!(matches!(
        progress::run(&handle, ())
            .on_progress(move |_| scopes.dispose())
            .await,
        Err(JobError::Worker(fusor_worker::WorkerError::OwnerDisposed))
    ));
}

#[fusor_worker::task]
pub async fn fetch_text(path: String, ctx: TaskContext) -> TaskResult<String, String> {
    fusor_async::fetch::get_text(&path, &ctx.cancellation_token())
        .await
        .map_err(|e| JobError::Application(e.to_string()))
}
#[fusor_worker::task]
pub fn echo(value: String) -> TaskResult<String> {
    Ok(value)
}
#[fusor_worker::task]
pub fn crash(_: ()) -> TaskResult<()> {
    panic!("intentional worker crash")
}
#[fusor_worker::task]
pub fn busy(milliseconds: u32, ctx: ComputeContext) -> TaskResult<u64> {
    let until = js_sys::Date::now() + f64::from(milliseconds);
    let mut iterations = 0;
    while js_sys::Date::now() < until {
        iterations += 1;
        ctx.check_cancelled()?;
    }
    Ok(iterations)
}
#[wasm_bindgen]
pub async fn exercise_failures() {
    stage("Fetch");
    use fusor_worker::WorkerError;
    let owner = fusor::Owner::new();
    owner.commit();
    let handle = owner.handle();
    assert!(matches!(
        fetch_text::run(&handle, "http://[".into()).await,
        Err(JobError::Application(_))
    ));
    assert_eq!(
        fetch_text::run(&handle, "sample.txt".into()).await.unwrap(),
        "worker fetch from app base\n"
    );
    let client = fusor_worker::spawn::<Delayed>(&handle, 0).await.unwrap();
    stage("Queue capacity");
    let mut active = client.change((1, 200));
    start(&mut active).await;
    let mut waiting = Vec::new();
    for _ in 0..64 {
        let mut call = client.change((1, 0));
        start(&mut call).await;
        waiting.push(call);
    }
    assert!(matches!(
        client.read(()).await,
        Err(JobError::Worker(WorkerError::QueueFull { capacity: 64 }))
    ));
    for call in &waiting {
        call.cancellation_handle().cancel();
    }
    assert_eq!(active.await.unwrap(), 2);
    assert_eq!(client.read(()).await.unwrap(), 2);
    stage("Resource integration");
    let scope = handle.clone();
    let resource = fusor_async::browser::resource(
        &handle,
        || Some(9),
        move |input, cancel| twice::run(&scope, input).cancel_on(&cancel),
    );
    for _ in 0..100 {
        if resource.with(|state| state.data().is_some()) {
            break;
        }
        pause(5).await;
    }
    assert!(resource.with(|state| state.data().is_some()));
    stage("Crash propagation");
    let crashed = crash::run(&handle, ()).await;
    stage(&format!("Crash result: {crashed:?}"));
    assert!(matches!(
        crashed,
        Err(JobError::Worker(WorkerError::Crashed { .. }))
    ));
    assert!(matches!(
        twice::run(&handle, 1).await,
        Err(JobError::Worker(WorkerError::Crashed { .. }))
    ));
}
#[wasm_bindgen]
pub async fn exercise_transport() {
    let owner = fusor::Owner::new();
    owner.commit();
    let handle = owner.handle();
    let data = echo::run(&handle, "x".repeat(1024 * 1024)).await.unwrap();
    assert_eq!(data.len(), 1024 * 1024);
    stage("CPU work");
    busy::run(&handle, 250).await.unwrap();
    stage("CPU done");
}

fn worker_failure(error: JobError<fusor_worker::NoError>) -> JsValue {
    JsValue::from_str(&error.to_string())
}

#[wasm_bindgen]
pub async fn startup_probe() -> Result<String, JsValue> {
    let owner = fusor::Owner::new();
    owner.commit();
    echo::run(&owner.handle(), "ready".into())
        .await
        .map_err(worker_failure)
}

#[wasm_bindgen]
pub async fn cancel_transport_failure() -> Result<u32, JsValue> {
    let owner = fusor::Owner::new();
    owner.commit();
    let service = fusor_worker::spawn::<Delayed>(&owner.handle(), 0)
        .await
        .map_err(worker_failure)?;
    let mut cancelled = service.change((1, 200));
    start(&mut cancelled).await;
    let mut waiting = service.read(());
    start(&mut waiting).await;
    cancelled.cancellation_handle().cancel();
    waiting.await.map_err(worker_failure)
}

#[wasm_bindgen]
pub async fn credit_transport_failure() -> Result<String, JsValue> {
    let owner = fusor::Owner::new();
    owner.commit();
    let mut stream = numbers::stream(&owner.handle(), 2).buffer(1);
    assert_eq!(stream.next().await.unwrap().unwrap(), 0);
    echo::run(&owner.handle(), "after credit".into())
        .await
        .map_err(worker_failure)
}
