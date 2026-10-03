//! Reuse JS string arguments for native DOM calls, without interning dynamic
//! application values or adding a cache lookup to every Wasm string conversion.
use super::HandlerId;
use crate::template::{self, ComponentId};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
use wasm_bindgen::{JsCast, prelude::*};
use web_sys::{Document, Element, EventTarget, NodeList};

// Compare against the live DOM in JavaScript. Returning its old string to Rust
// only to compare it allocates and transcodes a value that no caller needs.
#[wasm_bindgen(module = "/src/dom/strings.js")]
extern "C" {
    #[wasm_bindgen(js_name = setTextIfChanged)]
    pub(super) fn set_text_if_changed(node: &web_sys::Text, value: &str);
    #[wasm_bindgen(js_name = setIntegerTextIfChanged)]
    pub(super) fn set_integer_text_if_changed(node: &web_sys::Text, value: f64);
    #[wasm_bindgen(catch, js_name = setIntegerAttribute)]
    pub(super) fn set_integer_attribute(
        node: &Element,
        name: &str,
        value: f64,
    ) -> Result<(), JsValue>;
    #[wasm_bindgen(catch, js_name = serverRows)]
    fn server_rows_native(
        container: &Element,
        name: &JsValue,
        encoded: &str,
        count: u32,
        complete: bool,
    ) -> Result<js_sys::Array, JsValue>;
    // Native listeners stay on the JavaScript side, indexed by their handler
    // slot, so neither listening nor removal passes a callback handle.
    #[wasm_bindgen(js_name = listenBundleOk)]
    fn listen_bundle_ok(
        nodes: &JsValue,
        index: u32,
        name: &JsValue,
        dispatch: &JsValue,
        slot: u32,
        generation: u32,
    ) -> bool;
    // Removal reports success and keeps what it threw, so the common case
    // needs no exception wrapper.
    #[wasm_bindgen(js_name = unlistenBundleOk)]
    fn unlisten_bundle_ok(nodes: &JsValue, index: u32, name: &JsValue, slot: u32) -> bool;
    #[wasm_bindgen(js_name = unlistenOk)]
    fn unlisten_ok(target: &EventTarget, name: &JsValue, slot: u32) -> bool;
    #[wasm_bindgen(js_name = takeRemovalFailure)]
    fn take_removal_failure() -> JsValue;
    // Moving a row needs no handle to the moved node and rarely throws.
    #[wasm_bindgen(js_name = insertBeforeOk)]
    fn insert_before_ok(
        parent: &Element,
        node: &web_sys::Node,
        anchor: Option<&web_sys::Node>,
    ) -> bool;
    #[wasm_bindgen(js_name = listenOk)]
    fn listen_ok(
        target: &EventTarget,
        name: &JsValue,
        dispatch: &JsValue,
        slot: u32,
        generation: u32,
    ) -> bool;
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(extends = Element, js_name = Element)]
    type StringElement;
    #[wasm_bindgen(method, structural, js_name = getAttribute)]
    fn attribute(this: &StringElement, name: &JsValue) -> Option<String>;
    #[wasm_bindgen(method, structural, js_name = getAttribute)]
    fn attribute_value(this: &StringElement, name: &JsValue) -> JsValue;
    #[wasm_bindgen(method, structural, catch, js_name = setAttribute)]
    fn set_attribute_value(
        this: &StringElement,
        name: &JsValue,
        value: &JsValue,
    ) -> Result<(), JsValue>;

    #[wasm_bindgen(extends = Document, js_name = Document)]
    type StringDocument;
    #[wasm_bindgen(method, structural, catch, js_name = querySelectorAll)]
    fn query(this: &StringDocument, selector: &JsValue) -> Result<NodeList, JsValue>;

}

/// Framework names, in `NAMES` order.
#[derive(Clone, Copy)]
pub(super) enum Name {
    #[cfg_attr(
        not(feature = "islands"),
        expect(
            dead_code,
            reason = "component attributes are read during island hydration"
        )
    )]
    Component,
    Version,
    Element,
    Key,
    Click,
    Input,
    Change,
    Instance,
}

thread_local! {
    // These names are framework syntax. The cache cannot grow with input data.
    static NAMES: [JsValue; 8] = [
        template::COMPONENT_ATTRIBUTE,
        template::VERSION_ATTRIBUTE,
        template::ELEMENT_ATTRIBUTE,
        "data-fusor-key", "click", "input", "change", template::INSTANCE_ATTRIBUTE,
    ].map(JsValue::from_str);
}

fn with_name<R>(name: Name, call: impl FnOnce(&JsValue) -> R) -> R {
    NAMES.with(|names| call(&names[name as usize]))
}

fn string_element(element: &Element) -> &StringElement {
    element.unchecked_ref()
}

/// The first `count` element children of `container`, checked against the
/// newline-separated key encodings; `complete` also rejects any further row.
pub(super) fn server_rows(
    container: &Element,
    encoded: &str,
    count: u32,
    complete: bool,
) -> Result<js_sys::Array, JsValue> {
    with_name(Name::Key, |name| {
        server_rows_native(container, name, encoded, count, complete)
    })
}

pub(super) fn attribute(element: &Element, name: Name) -> Option<String> {
    with_name(name, |name| string_element(element).attribute(name))
}

pub(super) enum EventName {
    Cached(Name),
    Owned(JsValue),
}

impl From<&str> for EventName {
    fn from(name: &str) -> Self {
        match name {
            "click" => Self::Cached(Name::Click),
            "input" => Self::Cached(Name::Input),
            "change" => Self::Cached(Name::Change),
            _ => Self::Owned(JsValue::from_str(name)),
        }
    }
}

impl EventName {
    fn with<R>(&self, call: impl FnOnce(&JsValue) -> R) -> R {
        match self {
            Self::Cached(name) => with_name(*name, call),
            Self::Owned(value) => call(value),
        }
    }
}

/// Add a native listener that forwards its events to `dispatch` with the
/// handler's slot, returning the listener for removal.
pub(super) fn listen(
    target: &EventTarget,
    name: &EventName,
    dispatch: &JsValue,
    HandlerId { slot, generation }: HandlerId,
) -> Result<(), JsValue> {
    name.with(|name| status(listen_ok(target, name, dispatch, slot, generation)))
}

/// [`listen`] on a validated binding bundle entry.
pub(super) fn listen_bundle(
    nodes: &JsValue,
    index: u32,
    name: &EventName,
    dispatch: &JsValue,
    HandlerId { slot, generation }: HandlerId,
) -> Result<(), JsValue> {
    name.with(|name| {
        status(listen_bundle_ok(
            nodes, index, name, dispatch, slot, generation,
        ))
    })
}

pub(super) fn unlisten_bundle(
    nodes: &JsValue,
    index: u32,
    name: &EventName,
    slot: u32,
) -> Result<(), JsValue> {
    name.with(|name| status(unlisten_bundle_ok(nodes, index, name, slot)))
}

/// A `*Ok` host call's result: nothing, or what it threw.
fn status(ok: bool) -> Result<(), JsValue> {
    if ok {
        Ok(())
    } else {
        Err(take_removal_failure())
    }
}

/// `parent.insertBefore(node, anchor)` without returning the node.
pub(super) fn insert_before(
    parent: &Element,
    node: &web_sys::Node,
    anchor: Option<&web_sys::Node>,
) -> Result<(), JsValue> {
    status(insert_before_ok(parent, node, anchor))
}

pub(super) fn remove(target: &EventTarget, name: &EventName, slot: u32) -> Result<(), JsValue> {
    name.with(|name| status(unlisten_ok(target, name, slot)))
}

/// Bounded immutable metadata only; no DOM roots, scopes or application values.
pub(super) struct DescriptorStrings {
    component: ComponentId,
    version: u32,
    selector: JsValue,
    schema: JsValue,
    identity: JsValue,
}

thread_local! {
    static DESCRIPTORS: RefCell<VecDeque<Rc<DescriptorStrings>>> = const { RefCell::new(VecDeque::new()) };
}

pub(super) fn descriptor(component: ComponentId, version: u32) -> Rc<DescriptorStrings> {
    super::cached(
        &DESCRIPTORS,
        32,
        |entry| entry.component == component && entry.version == version,
        || {
            Rc::new(DescriptorStrings {
                component,
                version,
                selector: JsValue::from_str(&format!(
                    "[{}=\"{component}\"]",
                    template::COMPONENT_ATTRIBUTE
                )),
                schema: JsValue::from_str(&version.to_string()),
                identity: JsValue::from_str(&component.to_string()),
            })
        },
    )
}

impl DescriptorStrings {
    /// Root selector, schema version and component identity, as native strings.
    pub(super) fn native(&self) -> (&JsValue, &JsValue, &JsValue) {
        (&self.selector, &self.schema, &self.identity)
    }

    pub(super) fn roots(&self, document: &Document) -> Result<NodeList, JsValue> {
        document
            .unchecked_ref::<StringDocument>()
            .query(&self.selector)
    }

    pub(super) fn version_matches(&self, element: &Element) -> bool {
        with_name(Name::Version, |name| {
            string_element(element).attribute_value(name) == self.schema
        })
    }

    #[cfg(feature = "islands")]
    pub(super) fn component_matches(&self, element: &Element) -> bool {
        with_name(Name::Component, |name| {
            string_element(element).attribute_value(name) == self.identity
        })
    }

    pub(super) fn mark_instance(&self, element: &Element) -> Result<(), JsValue> {
        with_name(Name::Instance, |name| {
            string_element(element).set_attribute_value(name, &self.identity)
        })
    }
}

// Generated attribute names are static program metadata, never application
// values. Bound the shared registry; effects retain their immutable name if an
// entry is evicted.
thread_local! {
    static STATIC_ATTRIBUTES: RefCell<VecDeque<(&'static str, Rc<JsValue>)>> = const { RefCell::new(VecDeque::new()) };
}

pub(super) fn static_attribute(name: &'static str) -> Rc<JsValue> {
    super::cached(
        &STATIC_ATTRIBUTES,
        64,
        |(key, _)| *key == name,
        || (name, Rc::new(JsValue::from_str(name))),
    )
    .1
}
