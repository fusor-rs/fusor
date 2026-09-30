mod codec;
pub use codec::Codec;
pub(crate) use codec::RemoteLeases;

#[cfg(test)]
mod tests;

use crate::{Message, WorkerError, message::private::Sealed};
use serde::{Deserialize, Serialize};
use std::{fmt, marker::PhantomData, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedId {
    pub pool: String,
    pub generation: String,
    pub allocation: u64,
    // Diagnostic only: the UI and threaded worker may use different compilers.
    pub ty: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub enum Payload {
    Json(String),
    Shared(SharedId),
    Instance(u64),
}

/// A lease on an allocation in one pool. The allocation never lives on the UI.
pub struct Shared<T: Send + Sync + 'static> {
    lease: Arc<Lease>,
    ty: PhantomData<fn() -> T>,
}
struct Lease {
    id: SharedId,
    codec: Codec,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.codec.release(&self.id);
    }
}
impl<T: Send + Sync + 'static> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self {
            lease: Arc::clone(&self.lease),
            ty: PhantomData,
        }
    }
}
impl<T: Send + Sync + 'static> fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Shared").field(&self.lease.id).finish()
    }
}
impl<T: Send + Sync + 'static> Message for Shared<T> {}
impl<T: Send + Sync + 'static> Sealed for Shared<T> {
    fn encode(&self, limit: usize, codec: &Codec) -> Result<Payload, WorkerError> {
        codec.validate(&self.lease.id)?;
        let payload = Payload::Shared(self.lease.id.clone());
        crate::message::encode_json(&payload, limit)?;
        codec.retain(&self.lease.id)?;
        Ok(payload)
    }
    fn decode(payload: Payload, codec: &Codec) -> Result<Self, WorkerError> {
        let Payload::Shared(id) = payload else {
            return Err(WorkerError::SharedTypeMismatch);
        };
        codec.validate(&id)?;
        if let Err(error) = codec.validate_type::<T>(&id) {
            codec.release(&id);
            return Err(error);
        }
        Ok(Self {
            lease: Arc::new(Lease {
                id,
                codec: codec.clone(),
            }),
            ty: PhantomData,
        })
    }
}
