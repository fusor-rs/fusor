//! Bind real DOM nodes to Rust closures. The browser creates the HTML DOM.
//! A `Scope` owns its effects and event listeners. Drop it to unmount behavior.

pub mod application;
mod bindings;
mod branch;
mod children;
pub mod coherent;
mod commit;
mod component;
mod content;
#[doc(hidden)]
pub mod controls;
#[doc(hidden)]
pub use children::Children;
#[cfg(feature = "islands")]
pub mod delivery;
mod hydration;
mod keyed;
mod mount;
mod property;
mod range;
mod reconcile;
mod strings;
mod target;
#[doc(hidden)]
pub mod text_value;

pub use content::Content;
#[doc(hidden)]
pub use mount::{NestingGuard, TemplateNodes};
#[doc(hidden)]
pub use range::{Anchors, MountPoint};
pub use target::{ElementTarget, InputTarget};

use crate::{Effect, Owner, OwnerHandle, batch};
use std::{cell::RefCell, collections::VecDeque, rc::Rc, thread::LocalKey};
pub use wasm_bindgen::JsValue;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{Document, Element, Event, EventTarget, HtmlTemplateElement};

/// Implemented by the HTML build for each `rust:component="RustType"`.
/// State is shared by the generated closures without requiring `Clone`.
/// Keep the returned scope alive; dropping it releases every binding.
pub trait Component: Sized + 'static {
    const TEMPLATE_HASH: &'static str = "";
    const TEMPLATE_HTML: &'static str = "";
    fn mount(self) -> Result<Scope, JsValue>;

    /// Construct state after template validation, with its own weak lifetime.
    fn mount_with(make: impl FnOnce(OwnerHandle) -> Self) -> Result<Scope, JsValue> {
        Self::try_mount_with(|owner| Ok(make(owner)))
    }

    fn try_mount_with(
        make: impl FnOnce(OwnerHandle) -> Result<Self, JsValue>,
    ) -> Result<Scope, JsValue> {
        let scope = Self::prepare_component(None, Box::new(make))?;
        scope.try_commit()?;
        Ok(scope)
    }

    /// Prepare a child while owner-activation registrations wait. Ordinary
    /// constructor effects keep their immediate timing. Attach, then commit the
    /// returned scope; dropping it rolls back preparation. Used by outlets.
    fn prepare(
        parent: &OwnerHandle,
        make: impl FnOnce(OwnerHandle) -> Result<Self, JsValue>,
    ) -> Result<Scope, JsValue> {
        Self::prepare_component(Some(parent), Box::new(make))
    }

    /// Generated implementations validate the template before calling `make`.
    /// The default supports hand-written components that implement `mount`.
    #[doc(hidden)]
    fn prepare_component(
        parent: Option<&OwnerHandle>,
        make: ComponentFactory<'_, Self>,
    ) -> Result<Scope, JsValue> {
        let owner = parent.map(Owner::child).unwrap_or_default();
        let mut scope = make(owner.handle())?.mount()?;
        // A hand-written mount can return a previously prepared scope. Preserve
        // its original hydration adoption decision when replacing that owner.
        if let Some(owned) = scope.hydration_ownership.value(&scope.owner) {
            scope.hydration_ownership = HydrationOwnership::Preserved(owned);
        }
        scope.owner = owner;
        scope.prepare_queue(parent);
        Ok(scope)
    }
}

/// Erase the factory at the generated-code boundary so recursive components do
/// not recursively instantiate a different closure type at every nesting level.
#[doc(hidden)]
pub type ComponentFactory<'a, C> = Box<dyn FnOnce(OwnerHandle) -> Result<C, JsValue> + 'a>;

/// A reusable component backed by a cloned HTML template, rather than existing DOM.
/// Generated for types declared with `<template rust:component="Type">`.
pub trait TemplateComponent: Component {}

/// Compatibility import for the renderer-independent construction contract.
pub use crate::FromInputs;

/// Convert a component construction error at the browser mounting boundary.
///
/// Implement this for a custom portable error when enabling browser rendering.
/// There is deliberately no blanket `Display` implementation: JavaScript values
/// retain their original identity, and applications choose how to represent a
/// native error to the browser.
pub trait IntoMountError {
    fn into_mount_error(self) -> JsValue;
}

impl IntoMountError for JsValue {
    fn into_mount_error(self) -> JsValue {
        self
    }
}

impl IntoMountError for std::convert::Infallible {
    fn into_mount_error(self) -> JsValue {
        match self {}
    }
}

impl IntoMountError for String {
    fn into_mount_error(self) -> JsValue {
        JsValue::from_str(&self)
    }
}

impl IntoMountError for &str {
    fn into_mount_error(self) -> JsValue {
        JsValue::from_str(self)
    }
}

pub fn document() -> Result<Document, JsValue> {
    web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("fusor requires a browser document"))
}

/// Create an element using the browser's DOM API, with no markup macro.
pub fn element(tag: &str) -> Result<Element, JsValue> {
    document()?.create_element(tag)
}

fn missing(selector: &str) -> JsValue {
    JsValue::from_str(&format!("fusor: no element matches {selector:?}"))
}

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";

fn is_html(element: &Element) -> bool {
    element.namespace_uri().as_deref() == Some(HTML_NAMESPACE)
}

/// Replace a thread-local for the duration of `run`, restoring it on unwind.
fn scoped<T: 'static, R>(
    key: &'static LocalKey<RefCell<T>>,
    value: T,
    run: impl FnOnce() -> R,
) -> R {
    struct Restore<T: 'static>(&'static LocalKey<RefCell<T>>, Option<T>);
    impl<T: 'static> Drop for Restore<T> {
        fn drop(&mut self) {
            if let Some(previous) = self.1.take() {
                self.0.set(previous);
            }
        }
    }
    let _restore = Restore(key, Some(key.replace(value)));
    run()
}

/// Find or insert an entry in a small FIFO-bounded registry of static metadata.
/// No registry borrow crosses `make`, which may reenter through JavaScript.
fn cached<T: Clone + 'static>(
    registry: &'static LocalKey<RefCell<VecDeque<T>>>,
    limit: usize,
    matches: impl Fn(&T) -> bool,
    make: impl FnOnce() -> T,
) -> T {
    if let Some(found) =
        registry.with_borrow(|entries| entries.iter().rev().find(|entry| matches(entry)).cloned())
    {
        return found;
    }
    let entry = make();
    registry.with_borrow_mut(|entries| {
        if entries.len() >= limit {
            entries.pop_front();
        }
        entries.push_back(entry.clone());
    });
    entry
}

/// Prepare a component on its server-rendered root, when there is one.
fn with_native_root<R>(
    root: Option<&Element>,
    make: impl FnOnce() -> Result<R, JsValue>,
) -> Result<R, JsValue> {
    match root {
        #[cfg(feature = "islands")]
        Some(root) => hydration::with_root(root, make),
        _ => make(),
    }
}

/// Remove an owned root, first disposing islands activated inside it.
fn remove_tree(root: &Element) {
    #[cfg(feature = "islands")]
    delivery::dispose_tree(root);
    root.remove();
}

type Handler = Rc<dyn Fn(Event)>;

/// Listener handlers, reached from native listeners through one dispatcher.
/// Releasing a slot advances its generation, so a stale native listener
/// cannot reach a later handler that reuses the slot.
#[derive(Default)]
struct Handlers {
    slots: Vec<(u32, Option<Handler>)>,
    free: Vec<u32>,
}

impl Handlers {
    fn insert(&mut self, handler: Handler) -> (u32, u32) {
        if let Some(slot) = self.free.pop() {
            let entry = &mut self.slots[slot as usize];
            entry.1 = Some(handler);
            return (slot, entry.0);
        }
        self.slots.push((0, Some(handler)));
        ((self.slots.len() - 1) as u32, 0)
    }

    fn get(&self, slot: u32, generation: u32) -> Option<Handler> {
        let (current, handler) = self.slots.get(slot as usize)?;
        (*current == generation).then(|| handler.clone())?
    }

    /// The caller drops the handler after releasing the registry borrow.
    fn remove(&mut self, slot: u32, generation: u32) -> Option<Handler> {
        let entry = self
            .slots
            .get_mut(slot as usize)
            .filter(|(current, _)| *current == generation)?;
        entry.0 = entry.0.wrapping_add(1);
        self.free.push(slot);
        entry.1.take()
    }
}

thread_local! {
    static HANDLERS: RefCell<Handlers> = RefCell::new(Handlers::default());
    // No registry borrow is held while a handler runs: it may add, remove or
    // dispatch listeners, including its own.
    static DISPATCH: Closure<dyn Fn(u32, u32, Event)> = Closure::new(|slot, generation, event| {
        let handler = HANDLERS.with_borrow(|handlers| handlers.get(slot, generation));
        if let Some(handler) = handler {
            handler(event);
        }
    });
}

/// Where a listener is attached: a native target, or an entry of a validated
/// binding bundle, which keeps its original node.
enum ListenerTarget {
    Node(EventTarget),
    Bundle(Rc<JsValue>, u32),
}

/// A DOM event listener, removed when dropped.
pub struct Listener {
    target: ListenerTarget,
    event: strings::EventName,
    slot: u32,
    generation: u32,
}

impl Listener {
    /// Call `handler` for each `event` on `target`, as dispatched. Unlike
    /// [`Scope::on`], signal writes are not batched and no owner gates it.
    pub fn new(
        target: EventTarget,
        event: &str,
        handler: impl FnMut(Event) + 'static,
    ) -> Result<Self, JsValue> {
        Self::attach(ListenerTarget::Node(target), event, handler)
    }

    fn attach(
        target: ListenerTarget,
        event: &str,
        handler: impl FnMut(Event) + 'static,
    ) -> Result<Self, JsValue> {
        let handler = RefCell::new(handler);
        let handler: Handler = Rc::new(move |event| (handler.borrow_mut())(event));
        let (slot, generation) = HANDLERS.with_borrow_mut(|handlers| handlers.insert(handler));
        let event = strings::EventName::from(event);
        let listening = DISPATCH.with(|dispatch| match &target {
            ListenerTarget::Node(node) => {
                strings::listen(node, &event, dispatch.as_ref(), slot, generation)
            }
            ListenerTarget::Bundle(nodes, index) => {
                strings::listen_bundle(nodes, *index, &event, dispatch.as_ref(), slot, generation)
            }
        });
        match listening {
            Ok(()) => Ok(Self {
                target,
                event,
                slot,
                generation,
            }),
            Err(error) => {
                let handler =
                    HANDLERS.with_borrow_mut(|handlers| handlers.remove(slot, generation));
                drop(handler);
                Err(error)
            }
        }
    }

    /// Batch the handler's signal writes; skip events while `active` is false.
    fn batched(
        target: ListenerTarget,
        event: &str,
        active: impl Fn() -> bool + 'static,
        mut handler: impl FnMut(Event) + 'static,
    ) -> Result<Self, JsValue> {
        Self::attach(target, event, move |event| {
            if active() {
                batch(|| handler(event));
            }
        })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = match &self.target {
            ListenerTarget::Node(node) => strings::remove(node, &self.event, self.slot),
            ListenerTarget::Bundle(nodes, index) => {
                strings::unlisten_bundle(nodes, *index, &self.event, self.slot)
            }
        };
        let handler =
            HANDLERS.with_borrow_mut(|handlers| handlers.remove(self.slot, self.generation));
        drop(handler);
    }
}

/// A DOM island, including the lifetime of all of its reactive bindings.
/// Dropping the scope stops behavior; existing markup remains in the document.
#[must_use = "retain the scope to keep its DOM bindings and event listeners active"]
pub struct Scope {
    owner: Owner,
    root: Element,
    fragment: Option<MountPoint>,
    effects: Vec<Effect>,
    listeners: Vec<Listener>,
    children: Vec<Scope>,
    component_state: Option<Rc<dyn std::any::Any>>,
    retained: Vec<Box<dyn std::any::Any>>,
    mount_queue: Option<Rc<commit::CommitQueue>>,
    mount_parent: Option<OwnerHandle>,
    remove_on_drop: bool,
    render_tree: Option<Rc<coherent::Tree>>,
    hydrating: bool,
    hydration_ownership: HydrationOwnership,
}

// Generated prepared scopes read their final owner's monotone activation bit.
// The hand-written Component fallback can replace an already prepared owner;
// retain that previous adoption decision just as the old private marker did.
#[derive(Clone, Copy)]
enum HydrationOwnership {
    Untracked,
    CurrentOwner,
    Preserved(bool),
}
impl HydrationOwnership {
    fn value(self, owner: &Owner) -> Option<bool> {
        match self {
            Self::Untracked => None,
            Self::CurrentOwner => Some(owner.was_activated()),
            Self::Preserved(owned) => Some(owned),
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.owner.dispose();
        self.effects.clear();
        self.listeners.clear();
        self.children.clear();
        self.retained.clear();
        self.component_state.take();
        let hydrated_owned = self.hydration_ownership.value(&self.owner);
        if !self.hydrating || hydrated_owned == Some(true) {
            if let Some(fragment) = &self.fragment {
                fragment.remove();
            }
        }
        if self.remove_on_drop && hydrated_owned.unwrap_or(true) {
            remove_tree(&self.root);
        }
    }
}

impl crate::render::Scope for Scope {
    fn owner(&self) -> OwnerHandle {
        self.owner()
    }

    fn retain_state<T: 'static>(&mut self, state: T) -> Rc<T> {
        self.retain_state(state)
    }

    fn prepares_effects(&self) -> bool {
        self.prepares_effects()
    }
}

impl Scope {
    pub fn new(root: Element) -> Self {
        let owner = Owner::new();
        owner.commit();
        Self::with_owner(root, owner)
    }

    // Generated mounts allocate their final owner before resolution, but only
    // configure its integrations after the complete descriptor has validated.
    fn new_prepared(root: Element, parent: Option<&OwnerHandle>) -> Self {
        Self::with_owner(root, parent.map(Owner::child).unwrap_or_default())
    }

    fn with_owner(root: Element, owner: Owner) -> Self {
        Self {
            owner,
            root,
            fragment: None,
            effects: Vec::new(),
            listeners: Vec::new(),
            children: Vec::new(),
            component_state: None,
            retained: Vec::new(),
            mount_queue: None,
            mount_parent: None,
            remove_on_drop: false,
            render_tree: None,
            hydrating: false,
            hydration_ownership: HydrationOwnership::Untracked,
        }
    }

    /// Attach to existing, ordinary HTML.
    pub fn at(selector: &str) -> Result<Self, JsValue> {
        Ok(Self::new(
            document()?
                .query_selector(selector)?
                .ok_or_else(|| missing(selector))?,
        ))
    }

    /// Clone a standard HTML `<template>` containing exactly one root element.
    /// Templates contain plain HTML; Rust wires up the cloned elements.
    pub fn from_template(selector: &str) -> Result<Self, JsValue> {
        let template = document()?
            .query_selector(selector)?
            .ok_or_else(|| missing(selector))?
            .dyn_into::<HtmlTemplateElement>()
            .map_err(|_| JsValue::from_str("fusor: expected an HTML template"))?;
        Self::clone_template(&template)
    }

    fn clone_template(template: &HtmlTemplateElement) -> Result<Self, JsValue> {
        Ok(Self::new(Self::clone_template_root(template)?))
    }

    fn clone_template_root(template: &HtmlTemplateElement) -> Result<Element, JsValue> {
        let content = template.content();
        if content.child_element_count() != 1 {
            return Err(JsValue::from_str(
                "fusor: a row template needs exactly one root element",
            ));
        }
        Ok(content
            .first_element_child()
            .expect("one template element")
            .clone_node_with_deep(true)?
            .dyn_into::<Element>()?)
    }

    pub fn root(&self) -> &Element {
        &self.root
    }

    pub fn owner(&self) -> OwnerHandle {
        self.owner.handle()
    }

    /// Stop owned work immediately, even while an integration retains the scope.
    pub fn dispose(&self) {
        self.owner.dispose();
    }

    #[doc(hidden)]
    pub fn is_hydrating(&self) -> bool {
        self.hydrating
    }

    #[doc(hidden)]
    pub fn prepares_effects(&self) -> bool {
        #[cfg(feature = "islands")]
        if delivery::enabled() {
            return true;
        }
        self.is_coherent()
    }

    /// Activate prepared work after insertion succeeds. Ancestors must also commit.
    /// Logs setup failure and disposes the owner. Use [`Self::try_commit`] when
    /// the caller must propagate a setup error.
    pub fn commit(&self) {
        if let Err(error) = self.try_commit() {
            self.owner.dispose();
            web_sys::console::error_1(&error);
        }
    }

    #[doc(hidden)]
    pub fn prepare_owner(&mut self, parent: Option<&OwnerHandle>) {
        // A fresh owner resets readiness and invalidates any previous queued setup.
        self.owner = parent.map(Owner::child).unwrap_or_default();
        self.finish_owner_preparation(parent);
    }

    fn finish_owner_preparation(&mut self, parent: Option<&OwnerHandle>) {
        self.prepare_queue(parent);
        self.prepare_coherent(parent);
        #[cfg(feature = "islands")]
        delivery::prepare_preview_owner(&self.owner(), parent);
        if self.hydrating {
            // This used to be the first activation callback on the fresh owner.
            // Owner's monotone history records the same transition without a
            // per-scope Rc, callback registry entry and retained registration.
            self.hydration_ownership = HydrationOwnership::CurrentOwner;
        }
    }

    /// Retain an integration guard for this scope without replacing component state.
    pub fn retain(&mut self, guard: impl std::any::Any) {
        self.retained.push(Box::new(guard));
    }

    /// Insert a prepared view. Its root will be removed on drop. Does not commit.
    pub fn attach(&mut self, container: &Element) -> Result<(), JsValue> {
        container.append_child(&self.root)?;
        self.remove_on_drop = true;
        Ok(())
    }

    /// Keep component state alive even when its template has no dynamic bindings.
    #[doc(hidden)]
    pub fn retain_state<C: 'static>(&mut self, value: C) -> Rc<C> {
        let state = Rc::new(value);
        self.component_state = Some(state.clone());
        state
    }

    /// Adopt a component attached to existing markup, retaining its lifetime.
    pub fn adopt(&mut self, child: Scope) {
        child.commit();
        self.children.push(child);
    }

    /// Append a component and adopt its lifetime. Dropping the parent detaches
    /// its bindings and removes the mounted child's root from the document.
    pub fn mount_child(
        &mut self,
        target: impl ElementTarget,
        mut child: Scope,
    ) -> Result<(), JsValue> {
        target.resolve(self)?.append_child(&child.root)?;
        child.remove_on_drop = true;
        child.try_commit()?;
        self.children.push(child);
        Ok(())
    }

    /// Select within this island. `:scope` addresses the root itself.
    pub fn select(&self, selector: &str) -> Result<Element, JsValue> {
        if selector == ":scope" || self.root.matches(selector)? {
            return Ok(self.root.clone());
        }
        self.root
            .query_selector(selector)?
            .ok_or_else(|| missing(selector))
    }
}

#[cfg(test)]
mod handler_tests {
    use super::*;

    fn handler() -> Handler {
        Rc::new(|_| {})
    }

    #[test]
    fn released_slots_reject_stale_generations_and_are_reused() {
        let mut handlers = Handlers::default();
        let first = handler();
        let (slot, generation) = handlers.insert(first.clone());
        assert!(Rc::ptr_eq(&handlers.get(slot, generation).unwrap(), &first));
        assert!(handlers.get(slot, generation + 1).is_none());
        assert!(Rc::ptr_eq(
            &handlers.remove(slot, generation).unwrap(),
            &first
        ));
        assert!(handlers.get(slot, generation).is_none());
        assert!(handlers.remove(slot, generation).is_none());
        let second = handler();
        let (reused, next) = handlers.insert(second.clone());
        assert_eq!(reused, slot);
        assert_ne!(next, generation);
        assert!(handlers.get(slot, generation).is_none(), "stale listener");
        assert!(handlers.remove(slot, generation).is_none(), "stale removal");
        assert!(Rc::ptr_eq(&handlers.get(slot, next).unwrap(), &second));
        let (other, _) = handlers.insert(handler());
        assert_ne!(other, slot);
        assert!(handlers.get(99, 0).is_none());
    }
}
