use crate::bridge::{emit, start_worker};
use crate::{
    ComputeContext, Message, StreamSender, TaskContext, TaskResult, WorkerError,
    context::{ComputeBudget, Control},
    shared::{Codec, Payload},
};
use fusor_async::CancellationSource;
use std::{
    any::Any,
    cell::RefCell,
    collections::BTreeMap,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use wasm_bindgen::prelude::*;

pub type Invocation = Pin<Box<dyn Future<Output = TaskResult<Payload, Payload>>>>;
pub struct Registration {
    pub id: &'static str,
    pub pool: bool,
    pub kind: &'static str,
    pub invoke: fn(Vec<Payload>, Context) -> Invocation,
}
inventory::collect!(Registration);

pub struct Context {
    id: u64,
    instance: u64,
    batch_bytes: usize,
    control: Arc<Control>,
    source: Rc<CancellationSource>,
}
impl Context {
    pub fn codec(&self) -> Codec {
        self.control.codec.clone()
    }
    pub fn typed<P: Message>(&self) -> TaskContext<P> {
        TaskContext {
            control: Arc::clone(&self.control),
            source: Rc::clone(&self.source),
            marker: PhantomData,
        }
    }
    pub fn compute_context<P: Message>(&self) -> ComputeContext<P> {
        ComputeContext {
            control: Arc::clone(&self.control),
            marker: PhantomData,
        }
    }
    pub fn sender<T: Message>(&self) -> StreamSender<T> {
        StreamSender {
            id: self.id,
            limit: self.batch_bytes,
            control: Arc::clone(&self.control),
            marker: PhantomData,
        }
    }
    pub fn store<T: 'static>(&self, state: T) -> Payload {
        if !self.control.cancelled.load(Ordering::Acquire) {
            SERVICES.with(|services| {
                services.borrow_mut().insert(self.id, Box::new(state));
            });
        }
        Payload::Instance(self.id)
    }
    pub fn take<T: 'static>(&self) -> Result<Box<T>, WorkerError> {
        SERVICES
            .with(|services| services.borrow_mut().remove(&self.instance))
            .ok_or(WorkerError::Closed)?
            .downcast()
            .map_err(|_| WorkerError::IncompatibleArtifact)
    }
    pub fn restore<T: 'static>(&self, state: Box<T>) {
        SERVICES.with(|services| {
            services.borrow_mut().insert(self.instance, state);
        });
    }
}
type ActiveOperation = (Arc<Control>, Rc<CancellationSource>);
thread_local! {
    static COMPUTE: RefCell<Arc<ComputeBudget>> = RefCell::new(Arc::new(ComputeBudget { queue: Mutex::default(), threads: 1, capacity: 64 }));
    static CODEC: RefCell<Codec> = RefCell::new(Codec::default());
    static OPERATIONS: RefCell<BTreeMap<u64, ActiveOperation>> = const { RefCell::new(BTreeMap::new()) };
    static SERVICES: RefCell<BTreeMap<u64, Box<dyn Any>>> = const { RefCell::new(BTreeMap::new()) };
}

#[wasm_bindgen]
pub fn __fusor_worker_manifest() -> String {
    #[derive(serde::Serialize)]
    struct Entry {
        id: &'static str,
        pool: bool,
        kind: &'static str,
    }
    let mut entries: Vec<_> = inventory::iter::<Registration>
        .into_iter()
        .map(|r| Entry {
            id: r.id,
            pool: r.pool,
            kind: r.kind,
        })
        .collect();
    entries.sort_by_key(|r| r.id);
    serde_json::to_string(&(1, entries)).expect("worker registration")
}
#[wasm_bindgen]
pub fn __fusor_worker_initialize(
    pool: String,
    generation: String,
    threads: usize,
    capacity: usize,
) {
    COMPUTE.with(|budget| {
        *budget.borrow_mut() = Arc::new(ComputeBudget {
            queue: Mutex::default(),
            threads,
            capacity,
        })
    });
    if !pool.is_empty() {
        CODEC.with(|codec| *codec.borrow_mut() = Codec::local(pool, generation));
    }
}
#[wasm_bindgen]
pub async fn __fusor_worker_call(
    id: u64,
    entry: String,
    arguments: String,
    instance: u64,
    batch_bytes: usize,
) -> String {
    let result = invoke(id, &entry, &arguments, instance, batch_bytes).await;
    match crate::message::encode_json(&result, crate::message::MESSAGE_LIMIT) {
        Ok(encoded) => encoded,
        Err(error) => {
            CODEC.with(|codec| match result {
                Ok(payload) | Err(crate::JobError::Application(payload)) => {
                    codec.borrow().discard(payload)
                }
                _ => {}
            });
            serde_json::to_string(&TaskResult::<Payload, Payload>::Err(error.into()))
                .expect("worker error")
        }
    }
}
async fn invoke(
    id: u64,
    entry: &str,
    arguments: &str,
    instance: u64,
    batch_bytes: usize,
) -> TaskResult<Payload, Payload> {
    let registration = inventory::iter::<Registration>
        .into_iter()
        .find(|r| r.id == entry)
        .ok_or(WorkerError::IncompatibleArtifact)?;
    let arguments: Vec<Payload> =
        serde_json::from_str(arguments).map_err(|error| WorkerError::Decode {
            message: error.to_string(),
        })?;
    let codec = CODEC.with(|codec| codec.borrow().clone());
    let control = Arc::new(Control {
        cancelled: AtomicBool::new(false),
        progress: Mutex::new(None),
        codec,
        failure: Mutex::new(None),
        compute: COMPUTE.with(|budget| Arc::clone(&budget.borrow())),
    });
    let source = Rc::new(CancellationSource::default());
    OPERATIONS.with(|operations| {
        operations
            .borrow_mut()
            .insert(id, (Arc::clone(&control), Rc::clone(&source)));
    });
    let context = Context {
        id,
        instance,
        batch_bytes,
        control: Arc::clone(&control),
        source,
    };
    let result_control = control;
    let result = if registration.kind == "sync" && context.control.codec.is_pool() {
        run_sync(registration.invoke, arguments, &context).await
    } else {
        (registration.invoke)(arguments, context).await
    };
    __fusor_worker_tick();
    OPERATIONS.with(|operations| {
        operations.borrow_mut().remove(&id);
    });
    let failure = result_control.failure.lock().unwrap().take();
    if let Some(error) = failure {
        match result {
            Ok(payload) | Err(crate::JobError::Application(payload)) => {
                result_control.codec.discard(payload)
            }
            _ => {}
        }
        Err(error.into())
    } else {
        result
    }
}
#[wasm_bindgen]
pub fn __fusor_worker_cancel(id: u64) {
    let operation = OPERATIONS.with(|operations| operations.borrow().get(&id).cloned());
    if let Some((control, source)) = operation {
        control.cancelled.store(true, Ordering::Release);
        source.cancel();
    }
}
#[wasm_bindgen]
pub fn __fusor_worker_tick() {
    let updates: Vec<_> = OPERATIONS.with(|operations| {
        operations
            .borrow()
            .iter()
            .filter_map(|(&id, (control, _))| {
                let payload = control.progress.lock().unwrap().take()?;
                if control.cancelled.load(Ordering::Acquire) {
                    control.codec.discard(payload);
                    None
                } else {
                    Some((id, Arc::clone(control), payload))
                }
            })
            .collect()
    });
    for (id, control, payload) in updates {
        let event = crate::job::Event::Progress(payload);
        match crate::message::encode_json(&event, crate::message::MESSAGE_LIMIT) {
            Ok(encoded) => emit(id, &encoded),
            Err(error) => {
                if let crate::job::Event::Progress(payload) = event {
                    control.codec.discard(payload);
                }
                *control.failure.lock().unwrap() = Some(error);
            }
        }
    }
}
#[wasm_bindgen]
pub fn __fusor_worker_drop_service(instance: u64) {
    let service = SERVICES.with(|services| services.borrow_mut().remove(&instance));
    drop(service);
}
#[wasm_bindgen]
pub fn __fusor_worker_lease(retain: bool, shared: String) -> Result<(), JsValue> {
    let id =
        serde_json::from_str(&shared).map_err(|error| JsValue::from_str(&error.to_string()))?;
    CODEC.with(|codec| {
        let codec = codec.borrow();
        if retain {
            codec
                .retain(&id)
                .map_err(|error| JsValue::from_str(&error.to_string()))
        } else {
            codec.release(&id);
            Ok(())
        }
    })
}

pub(crate) async fn compute<
    P: Message,
    R: Send + 'static,
    F: FnOnce(ComputeContext<P>) -> R + Send + 'static,
>(
    context: ComputeContext<P>,
    work: F,
    bounded: bool,
) -> Result<R, WorkerError> {
    #[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
    {
        let budget = Arc::clone(&context.control.compute);
        if !context.control.codec.is_pool() {
            return Err(WorkerError::PoolRequired);
        }
        let (send, receive) = futures_channel::oneshot::channel();
        budget.submit(
            Box::new(move || {
                let result = context.check_cancelled().map(|_| work(context));
                let _ = send.send(result);
            }),
            bounded,
        )?;
        receive.await.map_err(|_| WorkerError::Terminated)?
    }
    #[cfg(not(all(target_arch = "wasm32", target_feature = "atomics")))]
    {
        let _ = (context, work, bounded);
        Err(WorkerError::PoolRequired)
    }
}

#[wasm_bindgen]
pub fn __fusor_worker_boot(app: JsValue, configuration: String) {
    start_worker(app, &configuration);
}

async fn run_sync(
    invoke: fn(Vec<Payload>, Context) -> Invocation,
    arguments: Vec<Payload>,
    context: &Context,
) -> TaskResult<Payload, Payload> {
    let id = context.id;
    let instance = context.instance;
    let batch_bytes = context.batch_bytes;
    let control = Arc::clone(&context.control);
    let arguments = crate::__private::Arguments::from_values(arguments, &context.codec());
    compute(
        context.compute_context::<()>(),
        move |_| {
            use futures_util::FutureExt;
            let context = Context {
                id,
                instance,
                batch_bytes,
                control,
                source: Rc::new(CancellationSource::default()),
            };
            invoke(arguments.finish(), context)
                .now_or_never()
                .expect("synchronous task adapter never yields")
        },
        false,
    )
    .await?
}

#[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
impl ComputeBudget {
    fn submit(
        self: &Arc<Self>,
        work: Box<dyn FnOnce() + Send>,
        bounded: bool,
    ) -> Result<(), WorkerError> {
        let mut queue = self.queue.lock().unwrap();
        if queue.active < self.threads {
            queue.active += 1;
            drop(queue);
            self.launch(work);
        } else {
            if bounded && queue.bounded == self.capacity {
                return Err(WorkerError::QueueFull {
                    capacity: self.capacity,
                });
            }
            queue.bounded += usize::from(bounded);
            queue.waiting.push_back((bounded, work));
        }
        Ok(())
    }
    fn launch(self: &Arc<Self>, work: Box<dyn FnOnce() + Send>) {
        let executor = Arc::clone(self);
        rayon::spawn(move || {
            work();
            let next = {
                let mut queue = executor.queue.lock().unwrap();
                match queue.waiting.pop_front() {
                    Some((bounded, work)) => {
                        queue.bounded -= usize::from(bounded);
                        Some(work)
                    }
                    None => {
                        queue.active -= 1;
                        None
                    }
                }
            };
            if let Some(work) = next {
                executor.launch(work);
            }
        });
    }
}
