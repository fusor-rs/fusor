use derive_where::derive_where;
use fusor_async::ResourceData;
use std::{num::NonZeroUsize, rc::Rc, time::Duration};

#[derive(Clone, Copy, Debug)]
pub enum Freshness {
    For(Duration),
    Forever,
}
impl Freshness {
    pub(crate) fn is_fresh(self, age: Duration) -> bool {
        match self {
            Self::For(limit) => age < limit,
            Self::Forever => true,
        }
    }
}

/// Every cache policy is explicit. Capacity bounds entry count (not value bytes).
/// Active entries are never silently evicted; new keys report `Capacity` when all
/// entries are pinned. Retry that subscription with `Query::refresh` after release.
#[derive(Clone, Copy, Debug)]
pub struct QueryOptions {
    pub freshness: Freshness,
    pub retention: Duration,
    pub capacity: NonZeroUsize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheInfo {
    pub entries: usize,
    pub observers: usize,
}

#[derive(Debug)]
#[derive_where(Clone; K)]
pub enum QueryState<K, T, E> {
    Idle,
    Loading {
        key: K,
        previous: Option<ResourceData<K, T>>,
    },
    Ready(ResourceData<K, T>),
    Error {
        key: K,
        error: Rc<E>,
        previous: Option<ResourceData<K, T>>,
    },
    /// All cache slots are held by active subscriptions. No request was started.
    Capacity {
        key: K,
    },
    Disposed,
}
impl<K, T, E> QueryState<K, T, E> {
    pub fn data(&self) -> Option<&ResourceData<K, T>> {
        match self {
            Self::Ready(data) => Some(data),
            Self::Loading { previous, .. } | Self::Error { previous, .. } => previous.as_ref(),
            Self::Idle | Self::Capacity { .. } | Self::Disposed => None,
        }
    }
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading { .. })
    }
}
