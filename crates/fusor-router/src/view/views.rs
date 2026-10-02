//! How a boundary chooses its view. Routers differ only here.
use super::RouteScope;
use crate::AppUrl;
use fusor::OwnerHandle;
use std::any::Any;

/// A boundary's views: which one a URL selects, and how to build it.
pub(crate) trait Views<S: RouteScope> {
    /// The view for `url`, whose first `prefix` path segments belong to
    /// enclosing boundaries. `None` shows nothing.
    fn select(&self, url: &AppUrl, prefix: usize) -> Result<Option<Selection<'_, S>>, S::Error>;
    /// Whether a link to `url` belongs to this router, not a new document.
    #[cfg(feature = "browser")]
    fn accepts_link(&self, _url: &AppUrl) -> bool {
        true
    }
}

/// Builds a view as a prepared child of the given owner.
pub(crate) type Render<'a, S> =
    dyn FnOnce(&OwnerHandle) -> Result<S, <S as RouteScope>::Error> + 'a;

pub(crate) struct Selection<'a, S: RouteScope> {
    /// An unchanged identity keeps the mounted view.
    pub(crate) identity: Box<dyn Identity>,
    /// Path segments consumed through this view, where nested boundaries start.
    pub(crate) consumed: usize,
    pub(crate) render: Box<Render<'a, S>>,
}

pub(crate) trait Identity {
    fn as_any(&self) -> &dyn Any;
    fn same(&self, other: &dyn Identity) -> bool;
}

impl<T: PartialEq + 'static> Identity for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn same(&self, other: &dyn Identity) -> bool {
        other.as_any().downcast_ref::<T>() == Some(self)
    }
}
