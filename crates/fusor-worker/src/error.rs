use serde::{Deserialize, Serialize};
use std::{error::Error, fmt};

/// An operation with no application error.
#[derive(Debug, Serialize, Deserialize)]
pub enum NoError {}
impl fmt::Display for NoError {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {}
    }
}
impl Error for NoError {}

pub type TaskResult<T, E = NoError> = Result<T, JobError<E>>;

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum JobError<E> {
    Application(E),
    Worker(WorkerError),
}
impl<E> From<WorkerError> for JobError<E> {
    fn from(error: WorkerError) -> Self {
        Self::Worker(error)
    }
}
impl<E: fmt::Display> fmt::Display for JobError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Application(e) => e.fmt(f),
            Self::Worker(e) => e.fmt(f),
        }
    }
}
impl<E: Error + 'static> Error for JobError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(match self {
            Self::Application(e) => e,
            Self::Worker(e) => e,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    DedicatedWorkers,
    SharedMemory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum WorkerError {
    Cancelled,
    OwnerDisposed,
    Closed,
    Terminated,
    CloseTimedOut,
    Unsupported { capability: Capability },
    PoolRequired,
    IncompatibleArtifact,
    WrongPool,
    StaleShared,
    SharedTypeMismatch,
    QueueFull { capacity: usize },
    PayloadTooLarge { limit: usize, actual: usize },
    InvalidConfiguration { message: String },
    Encode { message: String },
    Decode { message: String },
    Load { message: String },
    Crashed { message: String },
}
impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("worker operation cancelled"),
            Self::OwnerDisposed => f.write_str("worker owner disposed"),
            Self::Closed => f.write_str("worker is closed"),
            Self::Terminated => f.write_str("worker runtime terminated"),
            Self::CloseTimedOut => f.write_str("worker close timed out"),
            Self::Unsupported { capability: Capability::SharedMemory } => {
                f.write_str("shared memory unavailable; serve over HTTPS with Cross-Origin-Opener-Policy: same-origin and Cross-Origin-Embedder-Policy: require-corp in a browser supporting Wasm threads")
            }
            Self::Unsupported { capability } => {
                write!(f, "browser does not support {capability:?}")
            }
            Self::PoolRequired => f.write_str("operation requires .on(&pool)"),
            Self::IncompatibleArtifact => {
                f.write_str("worker artifact is incompatible; rebuild the application")
            }
            Self::WrongPool => f.write_str("shared handle belongs to another pool"),
            Self::StaleShared => f.write_str("shared handle has expired"),
            Self::SharedTypeMismatch => f.write_str("shared allocation has a different type"),
            Self::QueueFull { capacity } => write!(f, "worker queue is full (capacity {capacity})"),
            Self::PayloadTooLarge { limit, actual } => {
                write!(f, "worker payload is {actual} bytes; limit is {limit}")
            }
            Self::InvalidConfiguration { message } => {
                write!(f, "invalid worker configuration: {message}")
            }
            Self::Encode { message } => write!(f, "worker encoding failed: {message}"),
            Self::Decode { message } => write!(f, "worker decoding failed: {message}"),
            Self::Load { message } => write!(f, "worker loading failed: {message}"),
            Self::Crashed { message } => write!(f, "worker crashed: {message}"),
        }
    }
}
impl Error for WorkerError {}

pub(crate) fn configuration(message: impl Into<String>) -> WorkerError {
    WorkerError::InvalidConfiguration {
        message: message.into(),
    }
}
