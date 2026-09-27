//! Reuse JS string arguments for native DOM calls, without interning dynamic
//! application values or adding a cache lookup to every Wasm string conversion.
use crate::template::{self, ComponentId};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
use wasm_bindgen::{JsCast, prelude::*};
use web_sys::{Document, Element, EventTarget, NodeList};

// Compare against the live DOM in JavaScript. Returning its old string to Rust
// only to compare it allocates and transcodes a value that no caller needs.
#[wasm_bindgen(
    inline_js = "export function setTextIfChanged(node, value) { if (node.data !== value) node.data = value; } export function setIntegerTextIfChanged(node, number) { const value = '' + number; if (node.data !== value) node.data = value; } export function setIntegerAttribute(node, name, number) { node.setAttribute(name, '' + number); } export function listen(target, name, dispatch, slot, generation) { const listener = event => dispatch(slot, generation, event); target.addEventListener(name, listener); return listener; }"
)]
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
    #[wasm_bindgen(catch, js_name = listen)]
    fn listen_native(
        target: &EventTarget,
        name: &JsValue,
        dispatch: &JsValue,
        slot: u32,
        generation: u32,
    ) -> Result<JsValue, JsValue>;
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

    #[wasm_bindgen(extends = EventTarget, js_name = EventTarget)]
    type StringTarget;
    #[wasm_bindgen(method, structural, catch, js_name = removeEventListener)]
    fn remove(this: &StringTarget, name: &JsValue, callback: &JsValue) -> Result<(), JsValue>;
}

/// Framework names, in `NAMES` order.
#[derive(Clone, Copy)]
pub(super) enum Name {
    #[cfg_attr(not(feature = "islands"), allow(dead_code))]
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
    slot: u32,
    generation: u32,
) -> Result<JsValue, JsValue> {
    name.with(|name| listen_native(target, name, dispatch, slot, generation))
}

pub(super) fn remove(
    target: &EventTarget,
    name: &EventName,
    callback: &JsValue,
) -> Result<(), JsValue> {
    name.with(|name| {
        target
            .unchecked_ref::<StringTarget>()
            .remove(name, callback)
    })
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
