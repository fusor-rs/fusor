//! Owned background work, compiled and packaged with the application.
mod backend;
mod bridge;
mod context;
mod endpoint;
mod error;
mod job;
mod message;
mod pool;
mod service;
mod shared;
mod stream;
pub use context::{ComputeContext, TaskContext};
pub use endpoint::{Capabilities, capabilities};
pub use error::{Capability, JobError, NoError, TaskResult, WorkerError};
pub use fusor_worker_macros::{task, worker};
pub use job::{Bound, CancellationHandle, Job, Unbound};
pub use message::Message;
pub use pool::{Pool, PoolInit};
pub use service::{Close, Spawn, Worker, spawn};
pub use shared::Shared;
pub use stream::{ResultStream, StreamSender};

#[doc(hidden)]
pub mod __private {
    pub use crate::backend::{Context, Invocation, Registration};
    pub use crate::service::Service;
    pub use crate::shared::{Codec, Payload};
    use crate::{Job, JobError, Message, ResultStream, TaskResult, Unbound, WorkerError};
    pub use fusor::OwnerHandle;
    pub use fusor_worker_macros::__worker_method;
    pub use inventory;
    use std::marker::PhantomData;
    pub use wasm_bindgen;
    #[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
    pub use wasm_bindgen_rayon::init_thread_pool;
    pub struct Arguments {
        codec: Codec,
        values: Vec<Payload>,
    }
    impl Arguments {
        pub fn new(codec: &Codec) -> Self {
            Self {
                codec: codec.clone(),
                values: Vec::new(),
            }
        }
        pub(crate) fn from_values(values: Vec<Payload>, codec: &Codec) -> Self {
            Self {
                codec: codec.clone(),
                values,
            }
        }
        pub fn push<T: Message>(&mut self, value: &T) -> Result<(), WorkerError> {
            self.values.push(encode(value, &self.codec)?);
            Ok(())
        }
        pub fn finish(mut self) -> Vec<Payload> {
            std::mem::take(&mut self.values)
        }
    }
    impl Drop for Arguments {
        fn drop(&mut self) {
            for value in self.values.drain(..) {
                self.codec.discard(value);
            }
        }
    }
    pub struct Inputs {
        codec: Codec,
        values: std::vec::IntoIter<Payload>,
    }
    impl Inputs {
        pub fn new(values: Vec<Payload>, codec: Codec) -> Self {
            Self {
                codec,
                values: values.into_iter(),
            }
        }
        pub fn take<T: Message>(&mut self) -> Result<T, WorkerError> {
            decode(
                self.values
                    .next()
                    .ok_or(WorkerError::IncompatibleArtifact)?,
                &self.codec,
            )
        }
        pub fn finish(self) -> Result<(), WorkerError> {
            if self.values.len() == 0 {
                Ok(())
            } else {
                Err(WorkerError::IncompatibleArtifact)
            }
        }
    }
    impl Drop for Inputs {
        fn drop(&mut self) {
            for value in self.values.by_ref() {
                self.codec.discard(value);
            }
        }
    }
    pub fn encode<T: Message>(value: &T, codec: &Codec) -> Result<Payload, WorkerError> {
        value.encode(crate::message::MESSAGE_LIMIT, codec)
    }
    pub fn decode<T: Message>(payload: Payload, codec: &Codec) -> Result<T, WorkerError> {
        T::decode(payload, codec)
    }
    pub fn encode_result<T: Message, E: Message>(
        result: TaskResult<T, E>,
        codec: &Codec,
    ) -> TaskResult<Payload, Payload> {
        match result {
            Ok(value) => Ok(encode(&value, codec)?),
            Err(JobError::Application(error)) => Err(JobError::Application(encode(&error, codec)?)),
            Err(JobError::Worker(error)) => Err(error.into()),
        }
    }
    pub fn job<T: Message, E: Message, P: Message>(
        owner: &OwnerHandle,
        entry: &'static str,
        pool: bool,
        arguments: impl FnOnce(&Codec) -> Result<Vec<Payload>, WorkerError> + 'static,
    ) -> Job<T, E, P, Unbound> {
        Job {
            state: crate::job::State::new(owner, entry, pool, Box::new(arguments), false),
            marker: PhantomData,
        }
    }
    pub fn stream<T: Message, E: Message, P: Message>(
        owner: &OwnerHandle,
        entry: &'static str,
        pool: bool,
        arguments: impl FnOnce(&Codec) -> Result<Vec<Payload>, WorkerError> + 'static,
    ) -> ResultStream<T, E, P> {
        let job = job(owner, entry, pool, arguments);
        job.state.borrow_mut().stream = true;
        ResultStream { job }
    }
}
