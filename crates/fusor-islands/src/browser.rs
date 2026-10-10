//! The narrow Wasm entry and typed control surface. All loading and scheduling
//! belong to the shared JavaScript registry, not to a second Rust loader.
use crate::{Entry, Island, RenderMode, UnitWitness, attributes};
use fusor::{
    OwnerHandle, Registration,
    dom::{Component, Scope, delivery},
};
use serde::{Deserialize, de::IntoDeserializer};
use std::{cell::RefCell, collections::BTreeMap, marker::PhantomData, rc::Rc};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::Element;

mod attempt;
use attempt::{Attempt, Prepared, Preview};

type Make = dyn Fn(&Element, &str) -> Result<Prepared, JsValue>;
struct Factory {
    metadata: Entry,
    make: Box<Make>,
}

/// Content a replaced preview would lose: form controls and editable text.
const EDITABLE: &str = "input,textarea,select,[contenteditable]:not([contenteditable=false])";

/// A unit is initialized once, with a separate retained scope for each instance.
/// Registration installs metadata/factories without constructing application state.
pub struct Unit {
    entries: BTreeMap<String, Factory>,
    attempts: RefCell<BTreeMap<String, Rc<Attempt>>>,
}
impl Default for Unit {
    fn default() -> Self {
        // A delivery unit mounts from its own embedded templates.
        delivery::enable();
        Self {
            entries: BTreeMap::new(),
            attempts: RefCell::default(),
        }
    }
}
impl Unit {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn entry<D: Island, C: Component>(
        mut self,
        make: impl Fn(OwnerHandle, D::Props) -> C + 'static,
    ) -> Self {
        assert!(
            !self.entries.contains_key(D::NAME),
            "duplicate island entry {}",
            D::NAME
        );
        let metadata = Entry::new::<D>(C::TEMPLATE_HASH);
        let make = Box::new(move |host: &Element, text: &str| prepare::<D, C>(&make, host, text));
        self.entries
            .insert(D::NAME.into(), Factory { metadata, make });
        self
    }
    pub fn manifest(&self) -> String {
        crate::encode(&UnitWitness {
            version: crate::PROTOCOL_VERSION,
            entries: self
                .entries
                .values()
                .map(|entry| entry.metadata.clone())
                .collect(),
        })
        .expect("metadata is JSON")
    }
    pub fn activate(
        &self,
        descriptor: &str,
        host: &Element,
        props: &str,
        token: &str,
    ) -> Result<js_sys::Promise, JsValue> {
        if let Some(attempt) = self.attempts.borrow().get(token) {
            return Ok(attempt.promise());
        }
        let entry = self
            .entries
            .get(descriptor)
            .ok_or_else(|| JsValue::from_str("unknown unit entry"))?;
        let attempt = Attempt::new((entry.make)(host, props)?);
        self.attempts
            .borrow_mut()
            .insert(token.to_owned(), attempt.clone());
        attempt.start();
        Ok(attempt.promise())
    }
    pub fn dispose(&self, token: &str) {
        // Release the borrow first: disposal runs owner cleanup callbacks.
        let attempt = self.attempts.borrow_mut().remove(token);
        if let Some(attempt) = attempt {
            attempt.dispose();
        }
    }
}

/// Prepare one instance of `C` on `host` from its props text, once the server
/// is known to have rendered this schema and template.
fn prepare<D: Island, C: Component>(
    make: &impl Fn(OwnerHandle, D::Props) -> C,
    host: &Element,
    text: &str,
) -> Result<Prepared, JsValue> {
    if C::FRAGMENT {
        return Err(JsValue::from_str(
            "island entries require one native root element; wrap the fragment in a single-root component",
        ));
    }
    let props = crate::decode::<D::Props>(text)
        .map_err(|error| JsValue::from_str(&format!("island props: {error}")))?;
    if host.get_attribute(attributes::SCHEMA).as_deref() != Some(D::SCHEMA)
        || host.get_attribute(attributes::HASH).as_deref() != Some(C::TEMPLATE_HASH)
    {
        return Err(JsValue::from_str("island schema/template mismatch"));
    }
    let initial = initial_root(host)?;
    let prepare = || {
        let scope = C::prepare_component(None, Box::new(|owner| Ok(make(owner, props))))?;
        scope.root()?;
        Ok(scope)
    };
    match D::MODE {
        RenderMode::Attach => Ok(Prepared::Attach(delivery::with_root(&initial, prepare)?)),
        RenderMode::Preview => {
            if initial.matches(EDITABLE)? || initial.query_selector(EDITABLE)?.is_some() {
                return Err(JsValue::from_str(
                    "editable island previews cannot be replaced",
                ));
            }
            let (mut scope, readiness) = delivery::prepare_preview(prepare)?;
            // Keep the candidate detached, with ordinary owners still
            // prepared, until its initial coherent regions are ready.
            let staging = fusor::dom::document()?.create_element("div")?;
            scope.attach(&staging)?;
            let preview = Preview::new(host, initial, readiness, text)?;
            Ok(Prepared::Preview(scope, preview))
        }
    }
}

/// The server-rendered component root: the host's only element besides its
/// props script.
fn initial_root(host: &Element) -> Result<Element, JsValue> {
    let children = host.children();
    let mut roots = (0..children.length())
        .filter_map(|index| children.item(index))
        .filter(|node| !node.has_attribute(attributes::PROPS));
    match (roots.next(), roots.next()) {
        (Some(root), None) => Ok(root),
        _ => Err(JsValue::from_str(
            "an island requires exactly one initial component root",
        )),
    }
}

/// Export one unit with no application start function. The expression registers
/// typed factories; component construction happens only in `__fusor_activate`.
#[macro_export]
macro_rules! export {
    ($unit:expr) => {
        // wasm-bindgen rejects multiple start functions, including those emitted
        // by macros/dependencies. Reserve that native compiler slot with a pure
        // engine guard, so application startup cannot run eagerly in a unit.
        #[::wasm_bindgen::prelude::wasm_bindgen(start)]
        pub fn __fusor_delivery_start_guard() {}
        ::std::thread_local! { static __FUSOR_UNIT: ::std::cell::OnceCell<$crate::browser::Unit> = const { ::std::cell::OnceCell::new() }; }
        fn __fusor_unit<R>(read: impl FnOnce(&$crate::browser::Unit) -> R) -> R {
            __FUSOR_UNIT.with(|unit| read(unit.get_or_init(|| $unit)))
        }
        #[::wasm_bindgen::prelude::wasm_bindgen]
        pub fn __fusor_manifest() -> ::std::string::String {
            __fusor_unit(|unit| unit.manifest())
        }
        #[::wasm_bindgen::prelude::wasm_bindgen]
        pub fn __fusor_activate(descriptor: &str, host: &$crate::browser::IslandElement, props: &str, token: &str) -> ::std::result::Result<$crate::browser::IslandActivation, ::wasm_bindgen::JsValue> {
            __fusor_unit(|unit| unit.activate(descriptor, host, props, token))
        }
        #[::wasm_bindgen::prelude::wasm_bindgen]
        pub fn __fusor_dispose(token: &str) {
            __fusor_unit(|unit| unit.dispose(token));
        }
    };
}
#[doc(hidden)]
pub type IslandElement = Element;
#[doc(hidden)]
pub type IslandActivation = js_sys::Promise;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IslandStatus {
    Dormant,
    Requested,
    Binding,
    Active,
    Failed,
    Disposed,
}
#[derive(Clone, Debug, PartialEq)]
pub enum IslandError {
    Unavailable,
    UnknownInstance,
    DescriptorMismatch,
    InactiveOwner,
    DisposedOwner,
    StaleInstance,
    Cancelled,
    /// The unit failed to load, or the registry failed in another way.
    LoadFailed(RegistryError),
    BindingFailed(RegistryError),
}

/// Registry diagnostics retain the original JavaScript `Error.cause` value.
#[derive(Clone, Debug, PartialEq)]
pub struct RegistryError {
    pub message: String,
    pub cause: JsValue,
}
impl std::fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)?;
        if !self.cause.is_undefined() {
            write!(formatter, ": {:?}", self.cause)?;
        }
        Ok(())
    }
}
impl std::error::Error for RegistryError {}
impl std::fmt::Display for IslandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "the island registry is not available on this page",
            Self::UnknownInstance => "no island has this ID",
            Self::DescriptorMismatch => "the island's descriptor or props schema does not match",
            Self::InactiveOwner => "the requesting owner is not active yet",
            Self::DisposedOwner => "the requesting owner was disposed",
            Self::StaleInstance => "the island registration was removed or changed",
            Self::Cancelled => "the island request was cancelled",
            Self::LoadFailed(message) => return write!(f, "island failed to load: {message}"),
            Self::BindingFailed(message) => return write!(f, "island failed to bind: {message}"),
        })
    }
}
impl std::error::Error for IslandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::LoadFailed(error) | Self::BindingFailed(error) => Some(error),
            _ => None,
        }
    }
}

/// A typed property of a JavaScript object.
fn property<T: JsCast>(target: &JsValue, name: &str) -> Option<T> {
    js_sys::Reflect::get(target, &name.into())
        .ok()?
        .dyn_into()
        .ok()
}
fn text_property(target: &JsValue, name: &str) -> Option<String> {
    js_sys::Reflect::get(target, &name.into()).ok()?.as_string()
}

fn call(name: &str, args: &[JsValue]) -> Result<JsValue, IslandError> {
    let registry = property::<JsValue>(&js_sys::global(), "__fusor_islands")
        .ok_or(IslandError::Unavailable)?;
    let function = property::<js_sys::Function>(&registry, name).ok_or(IslandError::Unavailable)?;
    function
        .apply(&registry, &args.iter().collect())
        .map_err(decode_error)
}
fn decode_error(value: JsValue) -> IslandError {
    let message = text_property(&value, "message").unwrap_or_else(|| format!("{value:?}"));
    let error = RegistryError {
        message,
        cause: js_sys::Reflect::get(&value, &"cause".into()).unwrap_or_else(|error| error),
    };
    match text_property(&value, "code").as_deref() {
        Some("unknown-instance") => IslandError::UnknownInstance,
        Some("descriptor-mismatch") => IslandError::DescriptorMismatch,
        Some("stale-instance") => IslandError::StaleInstance,
        Some("cancelled") => IslandError::Cancelled,
        Some("binding-failed") => IslandError::BindingFailed(error),
        _ => IslandError::LoadFailed(error),
    }
}

fn live(owner: &OwnerHandle) -> Result<(), IslandError> {
    if owner.is_disposed() {
        return Err(IslandError::DisposedOwner);
    }
    Ok(())
}

/// Lookup performs no import, initialization or activation. The instance token
/// remains tied to this registration even if the DOM ID is later reused.
pub fn get<D: Island>(owner: &OwnerHandle, id: &str) -> Result<IslandRef<D>, IslandError> {
    live(owner)?;
    let token = call("lookup", &[id.into(), D::NAME.into(), D::SCHEMA.into()])?
        .as_string()
        .ok_or(IslandError::UnknownInstance)?;
    Ok(IslandRef {
        owner: owner.clone(),
        token,
        marker: PhantomData,
    })
}
pub struct IslandRef<D> {
    owner: OwnerHandle,
    token: String,
    marker: PhantomData<D>,
}
impl<D> Clone for IslandRef<D> {
    fn clone(&self) -> Self {
        Self {
            owner: self.owner.clone(),
            token: self.token.clone(),
            marker: PhantomData,
        }
    }
}
impl<D: Island> IslandRef<D> {
    pub fn status(&self) -> Result<IslandStatus, IslandError> {
        live(&self.owner)?;
        let status = call("status", &[self.token.as_str().into()])?
            .as_string()
            .ok_or(IslandError::StaleInstance)?;
        IslandStatus::deserialize(status.into_deserializer())
            .map_err(|_: serde::de::value::Error| IslandError::StaleInstance)
    }
    pub async fn prefetch(&self) -> Result<(), IslandError> {
        self.request("prefetch").await
    }
    pub async fn activate(&self) -> Result<(), IslandError> {
        self.request("activate").await
    }
    pub async fn retry(&self) -> Result<(), IslandError> {
        self.request("retry").await
    }
    async fn request(&self, action: &str) -> Result<(), IslandError> {
        live(&self.owner)?;
        if !self.owner.is_active() {
            return Err(IslandError::InactiveOwner);
        }
        let operation = call("request", &[self.token.as_str().into(), action.into()])?;
        let cancel =
            property::<js_sys::Function>(&operation, "cancel").ok_or(IslandError::Unavailable)?;
        // Cancel the operation if the owner is disposed, or this future is
        // dropped, before it settles.
        let on_cleanup = cancel.clone();
        let mut waiter = Waiter {
            cancel: Some(cancel),
            _registration: self.owner.on_cleanup(move || {
                let _ = on_cleanup.call0(&JsValue::UNDEFINED);
            }),
        };
        let promise =
            property::<js_sys::Promise>(&operation, "promise").ok_or(IslandError::Unavailable)?;
        let result = JsFuture::from(promise).await;
        waiter.cancel = None;
        live(&self.owner)?;
        result.map(|_| ()).map_err(decode_error)
    }
}
struct Waiter {
    cancel: Option<js_sys::Function>,
    _registration: Registration,
}
impl Drop for Waiter {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.call0(&JsValue::UNDEFINED);
        }
    }
}

fn event_name<D: Island>(event: &str) -> String {
    format!("fusor:{}:{event}", D::NAME)
}

/// Namespaced, ephemeral DOM events. The receiving scope guards its lifetime.
pub fn emit<D: Island, T: serde::Serialize>(
    host: &Element,
    event: &str,
    value: &T,
) -> Result<(), JsValue> {
    let options = web_sys::CustomEventInit::new();
    options.set_bubbles(true);
    options.set_detail(
        &crate::encode(value)
            .map_err(|error| JsValue::from_str(&error.to_string()))?
            .into(),
    );
    let event = web_sys::CustomEvent::new_with_event_init_dict(&event_name::<D>(event), &options)?;
    host.dispatch_event(&event)?;
    Ok(())
}

/// Decode an explicit cross-unit message while the receiving scope is active.
/// Payloads cross the boundary as JSON text, preserving Rust integer precision.
pub fn listen<D: Island, T: serde::de::DeserializeOwned + 'static>(
    scope: &mut Scope,
    target: &Element,
    event: &str,
    mut receive: impl FnMut(Result<T, crate::Error>) + 'static,
) -> Result<(), JsValue> {
    scope.on(target, &event_name::<D>(event), move |event| {
        let value = event
            .dyn_into::<web_sys::CustomEvent>()
            .ok()
            .and_then(|event| event.detail().as_string())
            .ok_or(crate::Error::MessagePayload)
            .and_then(|text| crate::decode(&text).map_err(crate::Error::Decode));
        receive(value);
    })
}
