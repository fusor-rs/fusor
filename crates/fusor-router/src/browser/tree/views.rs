//! How a boundary chooses its view. Routers differ only here.
use crate::AppUrl;
use fusor::{OwnerHandle, dom::Scope};
use std::any::Any;
use wasm_bindgen::JsValue;

/// A boundary's views: which one a URL selects, and how to build it.
pub(in crate::browser) trait Views {
    /// The view for `url`, whose first `prefix` path segments belong to
    /// enclosing boundaries. `None` shows nothing.
    fn select(&self, url: &AppUrl, prefix: usize) -> Result<Option<Selection<'_>>, JsValue>;
    /// Whether a link to `url` belongs to this router, not a new document.
    fn accepts_link(&self, _url: &AppUrl) -> bool {
        true
    }
}

/// Builds a view as a prepared child of the given owner.
pub(in crate::browser) type Render<'a> = dyn FnOnce(&OwnerHandle) -> Result<Scope, JsValue> + 'a;

pub(in crate::browser) struct Selection<'a> {
    /// An unchanged identity keeps the mounted view.
    pub(in crate::browser) identity: Box<dyn Identity>,
    /// Path segments consumed through this view, where nested boundaries start.
    pub(in crate::browser) consumed: usize,
    pub(in crate::browser) render: Box<Render<'a>>,
}

pub(in crate::browser) trait Identity {
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
