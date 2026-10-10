//! Per-Wasm template namespace and single-use native attachment roots.
use super::{JsValue, scoped};
use std::cell::{Cell, RefCell};
use wasm_bindgen::prelude::*;
use web_sys::Element;

#[wasm_bindgen(
    inline_js = "export function __fusor_dispose_tree(root) { globalThis.__fusor_islands?.disposeTree(root); }"
)]
extern "C" {
    #[wasm_bindgen(js_name = __fusor_dispose_tree)]
    pub(super) fn dispose_tree(root: &Element);
}

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
}

/// A delivery unit always uses its own embedded compiler templates. This flag
/// lives in that unit's Wasm memory and cannot affect other delivery units.
#[doc(hidden)]
pub fn enable() {
    ENABLED.set(true);
}
pub(crate) fn enabled() -> bool {
    ENABLED.get()
}

/// Attach one generated component to exactly this native root. Nested children
/// receive their own roots; the outer root is consumed before state construction.
#[doc(hidden)]
pub fn with_root<R>(
    root: &Element,
    make: impl FnOnce() -> Result<R, JsValue>,
) -> Result<R, JsValue> {
    super::hydration::with_root(root, make)
}

// A preview keeps its native HTML until every initially reached coherent region
// has published. The context follows prepared owners, including children added
// while another region is pending; it is sealed after the first publication.
struct PreviewContext;
impl crate::ContextKey for PreviewContext {
    type Value = PreviewReadiness;
}
struct PreviewState {
    boundaries: RefCell<Vec<crate::coherence::AsyncBoundary>>,
    revision: crate::Signal<usize>,
    closed: Cell<bool>,
}
#[doc(hidden)]
#[derive(Clone)]
pub struct PreviewReadiness(std::rc::Rc<PreviewState>);
impl PreviewReadiness {
    pub fn poll(&self) -> Result<bool, crate::coherence::Error> {
        self.0.revision.get();
        let boundaries = self.0.boundaries.borrow().clone();
        let mut ready = true;
        for boundary in boundaries {
            use crate::coherence::BoundaryStatus;
            match boundary.status() {
                BoundaryStatus::Ready | BoundaryStatus::Disposed => {}
                BoundaryStatus::Error(error) | BoundaryStatus::Faulted(error) => return Err(error),
                BoundaryStatus::Detached | BoundaryStatus::Pending => ready = false,
            }
        }
        Ok(ready)
    }
    pub fn close(&self) {
        self.0.closed.set(true);
        self.0.boundaries.borrow_mut().clear();
    }
}
thread_local! {
    static PREVIEW: RefCell<Option<PreviewReadiness>> = const { RefCell::new(None) };
}
#[doc(hidden)]
pub fn prepare_preview<R>(
    make: impl FnOnce() -> Result<R, JsValue>,
) -> Result<(R, PreviewReadiness), JsValue> {
    let readiness = PreviewReadiness(std::rc::Rc::new(PreviewState {
        boundaries: RefCell::new(Vec::new()),
        revision: crate::signal(0),
        closed: Cell::new(false),
    }));
    let value = scoped(&PREVIEW, Some(readiness.clone()), make)?;
    Ok((value, readiness))
}
pub(super) fn prepare_preview_owner(
    owner: &crate::OwnerHandle,
    parent: Option<&crate::OwnerHandle>,
) {
    if parent.is_some() {
        return;
    }
    if let Some(readiness) = PREVIEW.with_borrow(Clone::clone) {
        owner
            .provide::<PreviewContext>(readiness)
            .expect("fresh preview owner");
    }
}
pub(super) fn register_preview_boundary(
    owner: &crate::OwnerHandle,
    boundary: &crate::coherence::AsyncBoundary,
) {
    if let Some(readiness) = owner
        .context::<PreviewContext>()
        .filter(|readiness| !readiness.0.closed.get())
    {
        readiness.0.boundaries.borrow_mut().push(boundary.clone());
        readiness.0.revision.update(|revision| *revision += 1);
    }
}
