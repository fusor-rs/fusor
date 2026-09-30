use fusor::{OwnerHandle, Signal, signal};
use fusor_async::{Resource, ResourceState, browser::resource};
use fusor_worker::{ComputeContext, JobError, NoError, TaskResult};

#[fusor_worker::task]
pub fn count_primes(limit: u32, ctx: ComputeContext) -> TaskResult<u32> {
    let mut count = 0;
    for candidate in 2..limit {
        if candidate % 1024 == 0 {
            ctx.check_cancelled()?;
        }
        if !(2..)
            .take_while(|divisor| divisor * divisor <= candidate)
            .any(|divisor| candidate % divisor == 0)
        {
            count += 1;
        }
    }
    Ok(count)
}

struct App {
    limit: Signal<u32>,
    clicks: Signal<u32>,
    result: Resource<u32, u32, JobError<NoError>>,
}
impl App {
    fn new(owner: OwnerHandle) -> Self {
        let limit = signal(100_000);
        let selected = limit.clone();
        let task_owner = owner.clone();
        let result = resource(
            &owner,
            move || Some(selected.get()),
            move |limit, cancel| count_primes::run(&task_owner, limit).cancel_on(&cancel),
        );
        Self {
            limit,
            clicks: signal(0),
            result,
        }
    }
    fn status(&self) -> String {
        self.result.with(|state| match state {
            ResourceState::Ready(data) => format!("{} primes below {}", data.value, data.key),
            ResourceState::Error { error, .. } => error.to_string(),
            ResourceState::Loading { .. } => "Counting in a worker…".into(),
            _ => "Ready".into(),
        })
    }
}
fusor::template!("web/index.html");
