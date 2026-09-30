use crate::bridge::*;
use crate::{
    Capability, WorkerError,
    job::{Event, State},
    shared::{Codec, Payload, RemoteLeases},
};
use fusor::{ContextKey, Owner, OwnerHandle, Registration};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::{Rc, Weak},
    sync::{Arc, atomic::AtomicBool},
};
use wasm_bindgen::prelude::*;

pub struct Capabilities {
    pub dedicated_workers: bool,
    pub shared_memory: bool,
    pub hardware_parallelism: usize,
}
pub fn capabilities() -> Capabilities {
    #[cfg(target_arch = "wasm32")]
    {
        Capabilities {
            dedicated_workers: dedicated_workers(),
            shared_memory: shared_memory(),
            hardware_parallelism: hardware_parallelism().max(1),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Capabilities {
            dedicated_workers: false,
            shared_memory: false,
            hardware_parallelism: 1,
        }
    }
}

type ResponseCallback = Closure<dyn Fn(String)>;
pub(crate) struct Endpoint {
    runtime: JsValue,
    pub codec: Codec,
    pool: bool,
    next: Cell<u64>,
    jobs: RefCell<BTreeMap<u64, Weak<RefCell<State>>>>,
    error: RefCell<Option<WorkerError>>,
    callback: RefCell<Option<ResponseCallback>>,
    cleanup: RefCell<Option<Registration>>,
    services: RefCell<BTreeMap<u64, Registration>>,
    pub(crate) closing: Rc<RefCell<Option<js_sys::Promise>>>,
}
struct TaskRuntime {
    marker: Owner,
    endpoint: Rc<Endpoint>,
}
struct TaskRuntimeKey;
impl ContextKey for TaskRuntimeKey {
    type Value = TaskRuntime;
}

#[derive(serde::Deserialize)]
struct Response {
    id: u64,
    event: Event,
}
#[derive(serde::Serialize)]
#[serde(tag = "type")]
pub(crate) enum Command {
    Call {
        id: u64,
        entry: &'static str,
        instance: u64,
        stream: bool,
        capacity: usize,
        batch_bytes: usize,
        arguments: Vec<Payload>,
    },
    Cancel {
        id: u64,
    },
    Credit {
        id: u64,
    },
    Dispose {
        instance: u64,
    },
}
impl Endpoint {
    pub fn create(
        owner: &OwnerHandle,
        pool: bool,
        threads: usize,
        active: usize,
        capacity: usize,
    ) -> Result<Rc<Self>, WorkerError> {
        if owner.is_disposed() {
            return Err(WorkerError::OwnerDisposed);
        }
        let caps = capabilities();
        if !caps.dedicated_workers {
            return Err(WorkerError::Unsupported {
                capability: Capability::DedicatedWorkers,
            });
        }
        if pool && !caps.shared_memory {
            return Err(WorkerError::Unsupported {
                capability: Capability::SharedMemory,
            });
        }
        let holder: Rc<RefCell<Weak<Self>>> = Rc::new(RefCell::new(Weak::new()));
        let weak = Rc::clone(&holder);
        let callback: ResponseCallback = Closure::new(move |data: String| {
            if let Some(endpoint) = weak.borrow().upgrade() {
                endpoint.receive(&data);
            }
        });
        let runtime = open_runtime(
            pool,
            threads,
            active,
            capacity,
            callback.as_ref().unchecked_ref(),
        )
        .map_err(load_error)?;
        let remote = pool.then(|| {
            Arc::new(RemoteLeases {
                pool: pool_identity(&runtime),
                generation: generation(&runtime),
                alive: AtomicBool::new(true),
            })
        });
        let codec = remote
            .as_ref()
            .map(|remote| Codec::remote(Arc::clone(remote)))
            .unwrap_or_default();
        let endpoint = Rc::new(Self {
            runtime,
            codec,
            pool,
            next: Cell::new(0),
            jobs: RefCell::new(BTreeMap::new()),
            error: RefCell::new(None),
            callback: RefCell::new(Some(callback)),
            cleanup: RefCell::new(None),
            services: RefCell::new(BTreeMap::new()),
            closing: Rc::default(),
        });
        *holder.borrow_mut() = Rc::downgrade(&endpoint);
        let weak = Rc::downgrade(&endpoint);
        let cleanup = owner.on_cleanup(move || {
            if let Some(endpoint) = weak.upgrade() {
                endpoint.terminate(WorkerError::OwnerDisposed);
            }
        });
        *endpoint.cleanup.borrow_mut() = Some(cleanup);
        Ok(endpoint)
    }
    fn ordinary(owner: &OwnerHandle) -> Result<Rc<Self>, WorkerError> {
        if let Some(runtime) = owner.context::<TaskRuntimeKey>() {
            if runtime.marker.handle().is_child_of(owner) {
                return Ok(Rc::clone(&runtime.endpoint));
            }
        }
        let endpoint = Self::create(owner, false, 1, 1, 64)?;
        let marker = Owner::child(owner);
        marker.commit();
        owner
            .provide::<TaskRuntimeKey>(TaskRuntime {
                marker,
                endpoint: Rc::clone(&endpoint),
            })
            .map_err(|_| WorkerError::OwnerDisposed)?;
        Ok(endpoint)
    }
    pub fn submit(raw: &Rc<RefCell<State>>) -> Result<(), WorkerError> {
        let (endpoint, owner, dedicated, required) = {
            let state = raw.borrow();
            (
                state.endpoint.upgrade(),
                state.owner.clone(),
                state.dedicated,
                state.pool_required,
            )
        };
        let endpoint = match endpoint {
            Some(endpoint) => endpoint,
            None if raw.borrow().bound => return Err(WorkerError::Terminated),
            None if required => return Err(WorkerError::PoolRequired),
            None if dedicated => Self::create(&owner, false, 1, 1, 64)?,
            None => Self::ordinary(&owner)?,
        };
        if let Some(error) = endpoint.error.borrow().clone() {
            return Err(error);
        }
        if required && !endpoint.pool {
            return Err(WorkerError::PoolRequired);
        }
        let id = endpoint
            .next
            .get()
            .checked_add(1)
            .expect("worker job identity exhausted");
        endpoint.next.set(id);
        let encode = raw.borrow_mut().arguments.take().expect("job submits once");
        let arguments = encode(&endpoint.codec)?;
        if let Some(error) = raw.borrow().abort.clone() {
            for argument in arguments {
                endpoint.codec.discard(argument);
            }
            return Err(error);
        }
        let frame = {
            let mut state = raw.borrow_mut();
            state.endpoint = Rc::downgrade(&endpoint);
            state.id = Some(id);
            Command::Call {
                id,
                entry: state.entry,
                instance: state.instance,
                stream: state.stream,
                capacity: state.capacity,
                batch_bytes: state.batch_bytes,
                arguments,
            }
        };
        endpoint.jobs.borrow_mut().insert(id, Rc::downgrade(raw));
        if let Err(error) = endpoint.send(&frame) {
            if let Command::Call { arguments, .. } = frame {
                for argument in arguments {
                    endpoint.codec.discard(argument);
                }
            }
            endpoint.jobs.borrow_mut().remove(&id);
            return Err(error);
        }
        // A dedicated constructor must remain owned through initialization.
        if dedicated {
            raw.borrow_mut().keepalive = Some(endpoint);
        }
        Ok(())
    }
    fn receive(&self, data: &str) {
        let response = match serde_json::from_str::<Response>(data) {
            Ok(response) => response,
            Err(error) => {
                self.terminate(WorkerError::Decode {
                    message: error.to_string(),
                });
                return;
            }
        };
        if response.id == 0 {
            if let Event::Result(Err(crate::JobError::Worker(error))) = response.event {
                self.terminate(error);
            }
            return;
        }
        let terminal = matches!(response.event, Event::Result(_) | Event::End(_));
        let state = self.jobs.borrow().get(&response.id).and_then(Weak::upgrade);
        if terminal && !matches!(response.event, Event::End(_)) {
            self.jobs.borrow_mut().remove(&response.id);
        }
        if let Some(state) = state {
            State::receive(&state, response.event);
        } else {
            self.discard(response.event);
        }
    }
    pub fn discard(&self, event: Event) {
        match event {
            Event::Item(payload)
            | Event::Progress(payload)
            | Event::Result(Ok(payload))
            | Event::Result(Err(crate::JobError::Application(payload)))
            | Event::End(Err(crate::JobError::Application(payload))) => match payload {
                Payload::Instance(instance) => {
                    let _ = self.send(&Command::Dispose { instance });
                }
                payload => self.codec.discard(payload),
            },
            _ => {}
        }
    }
    fn send(&self, frame: &Command) -> Result<(), WorkerError> {
        let frame = crate::message::encode_json(frame, crate::message::MESSAGE_LIMIT)?;
        command(&self.runtime, &frame).map_err(load_error)
    }
    pub(crate) fn own_service(self: &Rc<Self>, instance: u64, owner: &OwnerHandle) {
        let weak = Rc::downgrade(self);
        let registration = owner.on_cleanup(move || {
            if let Some(endpoint) = weak.upgrade() {
                endpoint.services.borrow_mut().remove(&instance);
                endpoint.dispose(instance);
            }
        });
        self.services.borrow_mut().insert(instance, registration);
    }
    pub fn dispose(&self, instance: u64) {
        let _ = self.send(&Command::Dispose { instance });
    }
    pub fn cancel(&self, id: u64) {
        let _ = self.send(&Command::Cancel { id });
    }
    pub fn credit(&self, id: u64) {
        let _ = self.send(&Command::Credit { id });
    }
    pub fn forget(&self, id: u64) {
        self.jobs.borrow_mut().remove(&id);
    }
    pub fn terminate(&self, error: WorkerError) {
        if self.error.borrow().is_some() {
            return;
        }
        *self.error.borrow_mut() = Some(error.clone());
        self.codec.invalidate();
        shutdown(
            &self.runtime,
            &serde_json::to_string(&error).expect("worker error"),
        );
        let jobs = std::mem::take(&mut *self.jobs.borrow_mut());
        for state in jobs.into_values().filter_map(|state| state.upgrade()) {
            State::abort(&state, error.clone());
        }
    }
    pub async fn ready(&self) -> Result<(), WorkerError> {
        wasm_bindgen_futures::JsFuture::from(ready(&self.runtime))
            .await
            .map_err(load_error)?;
        self.error.borrow().clone().map_or(Ok(()), Err)
    }
    pub async fn close(
        &self,
        service: u64,
        timeout: std::time::Duration,
        closing: &RefCell<Option<js_sys::Promise>>,
    ) -> Result<(), WorkerError> {
        let promise = closing
            .borrow_mut()
            .get_or_insert_with(|| {
                close_runtime(&self.runtime, service, timeout.as_secs_f64() * 1000.0)
            })
            .clone();
        self.services.borrow_mut().remove(&service);
        wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map(|_| ())
            .map_err(load_error)
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.terminate(WorkerError::Terminated);
        self.callback.get_mut().take();
    }
}
fn load_error(value: JsValue) -> WorkerError {
    if let Some(error) = value
        .as_string()
        .and_then(|text| serde_json::from_str(&text).ok())
    {
        return error;
    }
    WorkerError::Load {
        message: value.as_string().unwrap_or_else(|| format!("{value:?}")),
    }
}
