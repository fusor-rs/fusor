use super::{Lease, Payload, Shared, SharedId};
use crate::WorkerError;
use std::{
    any::Any,
    collections::BTreeMap,
    marker::PhantomData,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

#[derive(Clone, Default)]
pub struct Codec {
    registry: Option<Arc<Mutex<Registry>>>,
    remote: Option<Arc<RemoteLeases>>,
}
pub(crate) struct RemoteLeases {
    pub pool: String,
    pub generation: String,
    pub alive: std::sync::atomic::AtomicBool,
}
impl RemoteLeases {
    fn update(&self, retain: bool, id: &SharedId) {
        #[cfg(target_arch = "wasm32")]
        crate::bridge::lease(
            &self.pool,
            retain,
            &serde_json::to_string(id).expect("shared lease"),
        );
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (retain, id);
    }
}
struct Allocation {
    value: Arc<dyn Any + Send + Sync>,
    leases: usize,
}
struct Registry {
    pool: String,
    generation: String,
    next: u64,
    alive: bool,
    allocations: BTreeMap<u64, Allocation>,
}

fn lock_registry(registry: &Mutex<Registry>) -> Result<MutexGuard<'_, Registry>, WorkerError> {
    registry.lock().map_err(|_| WorkerError::Crashed {
        message: "shared allocation registry was poisoned".into(),
    })
}
impl Codec {
    pub(crate) fn is_pool(&self) -> bool {
        self.registry.is_some() || self.remote.is_some()
    }
    pub(crate) fn local(pool: String, generation: String) -> Self {
        Self {
            registry: Some(Arc::new(Mutex::new(Registry {
                pool,
                generation,
                next: 0,
                alive: true,
                allocations: BTreeMap::new(),
            }))),
            remote: None,
        }
    }
    pub(crate) fn remote(remote: Arc<RemoteLeases>) -> Self {
        Self {
            registry: None,
            remote: Some(remote),
        }
    }
    pub(super) fn validate(&self, id: &SharedId) -> Result<(), WorkerError> {
        if let Some(registry) = &self.registry {
            let registry = lock_registry(registry)?;
            if registry.pool != id.pool {
                return Err(WorkerError::WrongPool);
            }
            if !registry.alive
                || registry.generation != id.generation
                || !registry.allocations.contains_key(&id.allocation)
            {
                return Err(WorkerError::StaleShared);
            }
        } else if let Some(remote) = &self.remote {
            if remote.pool != id.pool {
                return Err(WorkerError::WrongPool);
            }
            if !remote.alive.load(std::sync::atomic::Ordering::Acquire)
                || remote.generation != id.generation
            {
                return Err(WorkerError::StaleShared);
            }
        } else {
            return Err(WorkerError::PoolRequired);
        }
        Ok(())
    }
    pub(crate) fn retain(&self, id: &SharedId) -> Result<(), WorkerError> {
        self.validate(id)?;
        if let Some(registry) = &self.registry {
            let mut registry = lock_registry(registry)?;
            let allocation = registry
                .allocations
                .get_mut(&id.allocation)
                .ok_or(WorkerError::StaleShared)?;
            allocation.leases =
                allocation
                    .leases
                    .checked_add(1)
                    .ok_or_else(|| WorkerError::Crashed {
                        message: "shared allocation lease count exhausted".into(),
                    })?;
        } else if let Some(remote) = &self.remote {
            remote.update(true, id);
        }
        Ok(())
    }
    pub(super) fn validate_type<T: 'static>(&self, id: &SharedId) -> Result<(), WorkerError> {
        // Only the originating pool can check the concrete allocation. Remote
        // handles are opaque; type_name is neither unique nor compiler-stable.
        let Some(registry) = &self.registry else {
            return Ok(());
        };
        let registry = lock_registry(registry)?;
        let allocation = registry
            .allocations
            .get(&id.allocation)
            .ok_or(WorkerError::StaleShared)?;
        if allocation.value.is::<T>() {
            Ok(())
        } else {
            Err(WorkerError::SharedTypeMismatch)
        }
    }
    pub(crate) fn release(&self, id: &SharedId) {
        if let Some(registry) = &self.registry {
            let removed = {
                let mut registry = match registry.lock() {
                    Ok(registry) => registry,
                    Err(error) => {
                        drop(error);
                        self.invalidate();
                        return;
                    }
                };
                if registry.pool != id.pool || registry.generation != id.generation {
                    return;
                }
                let Some(value) = registry.allocations.get_mut(&id.allocation) else {
                    return;
                };
                value.leases -= 1;
                if value.leases == 0 {
                    registry.allocations.remove(&id.allocation)
                } else {
                    None
                }
            };
            drop(removed);
        } else if let Some(remote) = &self.remote {
            remote.update(false, id);
        }
    }
    pub(crate) fn discard(&self, payload: Payload) {
        if let Payload::Shared(id) = payload {
            self.release(&id);
        }
    }
    pub(crate) fn invalidate(&self) {
        if let Some(registry) = &self.registry {
            let values = {
                // Poisoned state is only recovered to retire the entire registry.
                let mut registry = registry.lock().unwrap_or_else(PoisonError::into_inner);
                registry.alive = false;
                std::mem::take(&mut registry.allocations)
            };
            drop(values);
        }
        if let Some(remote) = &self.remote {
            remote
                .alive
                .store(false, std::sync::atomic::Ordering::Release);
        }
    }
    pub(crate) fn share<T: Send + Sync + 'static>(
        &self,
        value: T,
    ) -> Result<Shared<T>, WorkerError> {
        let registry = self.registry.as_ref().ok_or(WorkerError::PoolRequired)?;
        let mut registry = lock_registry(registry)?;
        if !registry.alive {
            return Err(WorkerError::Closed);
        }
        registry.next = registry
            .next
            .checked_add(1)
            .ok_or_else(|| WorkerError::Crashed {
                message: "shared allocation identity exhausted".into(),
            })?;
        let allocation = registry.next;
        registry.allocations.insert(
            allocation,
            Allocation {
                value: Arc::new(value),
                leases: 1,
            },
        );
        let id = SharedId {
            pool: registry.pool.clone(),
            generation: registry.generation.clone(),
            allocation,
            ty: std::any::type_name::<T>().into(),
        };
        Ok(Shared {
            lease: Arc::new(Lease {
                id,
                codec: self.clone(),
            }),
            ty: PhantomData,
        })
    }
    pub(crate) fn resolve<T: Send + Sync + 'static>(
        &self,
        value: &Shared<T>,
    ) -> Result<Arc<T>, WorkerError> {
        self.validate(&value.lease.id)?;
        let registry = lock_registry(self.registry.as_ref().ok_or(WorkerError::PoolRequired)?)?;
        registry
            .allocations
            .get(&value.lease.id.allocation)
            .ok_or(WorkerError::StaleShared)?
            .value
            .clone()
            .downcast()
            .map_err(|_| WorkerError::SharedTypeMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn a_poisoned_registry_rejects_access_but_releases_its_allocations() {
        let value = Arc::new(());
        let codec = Codec::local("pool".into(), "generation".into());
        let shared = codec.share(Arc::clone(&value)).unwrap();
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                let _guard = codec.registry.as_ref().unwrap().lock().unwrap();
                panic!("interrupted registry mutation");
            }))
            .is_err()
        );
        let resolved = catch_unwind(AssertUnwindSafe(|| codec.resolve(&shared)));
        let allocated = catch_unwind(AssertUnwindSafe(|| codec.share(1_u32)));
        let invalidated = catch_unwind(AssertUnwindSafe(|| codec.invalidate()));
        let released = catch_unwind(AssertUnwindSafe(|| drop(shared)));
        assert!(matches!(resolved, Ok(Err(WorkerError::Crashed { .. }))));
        assert!(matches!(allocated, Ok(Err(WorkerError::Crashed { .. }))));
        assert!(invalidated.is_ok() && released.is_ok());
        assert_eq!(Arc::strong_count(&value), 1);
    }
}
