//! Keep the protocol scan next to the DOM for shapes with no managed subtrees.
//! The bounded registry owns descriptor metadata and inert certificates, never
//! application nodes. Typed resolution and native bundle binding share its plans.
#[cfg(feature = "islands")]
use super::{ElementHandle, Handles, Mounts, Resolution, Slot, TextPosition};
use super::{Scope, strings};
use crate::template::{ElementDescriptor, TemplateDescriptor, TextElementDescriptor, TextId};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
#[cfg(feature = "islands")]
use wasm_bindgen::JsCast;
use wasm_bindgen::{JsValue, prelude::wasm_bindgen};
#[cfg(feature = "islands")]
use web_sys::Text;
use web_sys::{Element, Event};

#[wasm_bindgen(module = "/src/dom/mount/flat.js")]
extern "C" {
    #[wasm_bindgen(js_name = flatPlan)]
    fn create_plan(
        elements: &js_sys::Array,
        tags: &js_sys::Array,
        texts: &js_sys::Array,
        text_elements: &js_sys::Array,
    ) -> JsValue;
    #[cfg(feature = "islands")]
    #[wasm_bindgen(catch, js_name = resolveFlat)]
    fn resolve_flat(plan: &JsValue, root: &Element) -> Result<js_sys::Array, JsValue>;
    #[wasm_bindgen(catch, js_name = resolveBindings)]
    fn resolve_bindings(plan: &JsValue, root: &Element, cached: bool) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = mountTemplateOk)]
    fn mount_template_ok(
        plan: &JsValue,
        selector: &JsValue,
        schema: &JsValue,
        identity: &JsValue,
        version_ok: bool,
    ) -> bool;
    /// The result of the last `*Ok` call, or what it threw.
    #[wasm_bindgen(js_name = takeOutcome)]
    fn take_outcome() -> JsValue;
    #[cfg(feature = "islands")]
    #[wasm_bindgen(js_name = hydrateRootOk)]
    fn hydrate_root_ok(
        plan: &JsValue,
        root: &Element,
        schema: &JsValue,
        identity: &JsValue,
        version_ok: bool,
    ) -> bool;
    #[wasm_bindgen(js_name = bindingText)]
    fn binding_text(nodes: &JsValue, index: u32, value: &str);
    #[wasm_bindgen(js_name = bindingIntegerText)]
    fn binding_integer_text(nodes: &JsValue, index: u32, value: f64);
    #[wasm_bindgen(js_name = bindingSetAttributeOk)]
    fn binding_set_attribute_ok(nodes: &JsValue, index: u32, name: &JsValue, value: &str) -> bool;
    #[wasm_bindgen(js_name = bindingSetIntegerAttributeOk)]
    fn binding_set_integer_attribute_ok(
        nodes: &JsValue,
        index: u32,
        name: &JsValue,
        value: f64,
    ) -> bool;
    #[wasm_bindgen(js_name = bindingRemoveAttributeOk)]
    fn binding_remove_attribute_ok(nodes: &JsValue, index: u32, name: &JsValue) -> bool;
    // All target types were validated before the user factory. Returning the
    // pinned handle must not re-check a prototype changed by that factory.
    #[wasm_bindgen(js_name = bindingElement)]
    fn binding_element(nodes: &JsValue, index: u32) -> Element;
}
struct Plan {
    elements: &'static [ElementDescriptor],
    texts: &'static [TextId],
    text_elements: &'static [TextElementDescriptor],
    native: JsValue,
}
thread_local! {
    static PLANS: RefCell<VecDeque<Rc<Plan>>> = const { RefCell::new(VecDeque::new()) };
}

fn plan(descriptor: &TemplateDescriptor) -> Rc<Plan> {
    let matches = |plan: &Rc<Plan>| {
        std::ptr::eq(plan.elements, descriptor.elements)
            && std::ptr::eq(plan.texts, descriptor.texts)
            && std::ptr::eq(plan.text_elements, descriptor.text_elements)
    };
    crate::dom::cached(&PLANS, 32, matches, || {
        let js = |value: &dyn std::fmt::Display| JsValue::from_str(&value.to_string());
        let elements = descriptor.elements.iter().map(|element| js(&element.id));
        let tags = descriptor
            .elements
            .iter()
            .map(|element| JsValue::from_str(element.tag));
        let texts = descriptor.texts.iter().map(|id| js(id));
        let text_elements = descriptor.text_elements.iter().flat_map(|text| {
            let host = text.host.map_or(JsValue::NULL, |host| js(&host));
            [js(&text.id), host, JsValue::from_str(text.tag)]
        });
        Rc::new(Plan {
            elements: descriptor.elements,
            texts: descriptor.texts,
            text_elements: descriptor.text_elements,
            native: create_plan(
                &elements.collect(),
                &tags.collect(),
                &texts.collect(),
                &text_elements.collect(),
            ),
        })
    })
}

pub(super) fn resolve_bundle(
    descriptor: &TemplateDescriptor,
    root: &Element,
    cached: bool,
) -> Result<JsValue, JsValue> {
    let plan = plan(descriptor);
    resolve_bindings(&plan.native, root, cached)
}

/// A `*Ok` host call's result: nothing, or what it threw.
fn outcome(ok: bool) -> Result<(), JsValue> {
    if ok { Ok(()) } else { Err(take_outcome()) }
}

fn binding_set_attribute(
    nodes: &JsValue,
    index: u32,
    name: &JsValue,
    value: &str,
) -> Result<(), JsValue> {
    outcome(binding_set_attribute_ok(nodes, index, name, value))
}

fn binding_set_integer_attribute(
    nodes: &JsValue,
    index: u32,
    name: &JsValue,
    value: f64,
) -> Result<(), JsValue> {
    outcome(binding_set_integer_attribute_ok(nodes, index, name, value))
}

fn binding_remove_attribute(nodes: &JsValue, index: u32, name: &JsValue) -> Result<(), JsValue> {
    outcome(binding_remove_attribute_ok(nodes, index, name))
}

/// Clone and resolve the document's template for a bundled flat descriptor in
/// one native call, returning the root and its binding bundle.
pub(super) fn mount_document_template(
    descriptor: &TemplateDescriptor,
) -> Result<(Element, JsValue), JsValue> {
    let plan = plan(descriptor);
    let metadata = strings::descriptor(descriptor.component, descriptor.version);
    let (selector, schema, identity) = metadata.native();
    let version_ok = descriptor.version == crate::template::VERSION;
    let mounted = mount_template_ok(&plan.native, selector, schema, identity, version_ok);
    let nodes = take_outcome();
    if !mounted {
        return Err(nodes);
    }
    let targets =
        descriptor.elements.len() + descriptor.texts.len() + descriptor.text_elements.len();
    let root = binding_element(&nodes, targets as u32);
    Ok((root, nodes))
}

/// Adopt a server-rendered root of a bundled flat descriptor in one native
/// call: identity, descriptor version, complete validation, then marking.
#[cfg(feature = "islands")]
pub(super) fn hydrate_root(
    descriptor: &TemplateDescriptor,
    root: &Element,
) -> Result<JsValue, JsValue> {
    let plan = plan(descriptor);
    let metadata = strings::descriptor(descriptor.component, descriptor.version);
    let (_, schema, identity) = metadata.native();
    let version_ok = descriptor.version == crate::template::VERSION;
    let hydrated = hydrate_root_ok(&plan.native, root, schema, identity, version_ok);
    let nodes = take_outcome();
    if hydrated { Ok(nodes) } else { Err(nodes) }
}

#[cfg(feature = "islands")]
pub(super) fn resolve(
    descriptor: &TemplateDescriptor,
    root: &Element,
) -> Result<Resolution, JsValue> {
    let plan = plan(descriptor);
    let nodes = resolve_flat(&plan.native, root)?;
    let mut handles = Handles::new();
    for (index, element) in descriptor.elements.iter().enumerate() {
        let node = nodes.get(index as u32);
        let handle = if element.tag == "input" {
            ElementHandle::Input(node.dyn_into()?)
        } else {
            ElementHandle::Element(node.dyn_into()?)
        };
        handles.insert(element.id, handle);
    }
    let existing = |index| -> Result<Option<Text>, JsValue> {
        let text = nodes.get(index);
        Ok(if text.is_null() {
            None
        } else {
            Some(text.dyn_into()?)
        })
    };
    let mut slots = Vec::with_capacity(descriptor.texts.len() + descriptor.text_elements.len());
    for (index, id) in descriptor.texts.iter().enumerate() {
        let offset = (descriptor.elements.len() + index * 3) as u32;
        slots.push(Slot {
            id: *id,
            position: TextPosition::Anchored {
                start: nodes.get(offset).dyn_into()?,
                end: nodes.get(offset + 1).dyn_into()?,
            },
            existing: existing(offset + 2)?,
        });
    }
    for (index, expected) in descriptor.text_elements.iter().enumerate() {
        let offset = (descriptor.elements.len() + descriptor.texts.len() * 3 + index * 2) as u32;
        slots.push(Slot {
            id: expected.id,
            position: TextPosition::Element(nodes.get(offset).dyn_into()?),
            existing: existing(offset + 1)?,
        });
    }
    handles.finish();
    Ok((handles, slots, Mounts::new()))
}

impl Scope {
    #[doc(hidden)]
    pub fn bundle_text_node_value(
        &mut self,
        nodes: &Rc<JsValue>,
        index: u32,
        read: impl Fn() -> crate::dom::text_value::Output + 'static,
    ) -> Result<(), JsValue> {
        use crate::dom::text_value::Output;
        let nodes = Rc::clone(nodes);
        self.bind_dom_infallible(move || match read() {
            Output::String(value) => binding_text(&nodes, index, &value),
            Output::Integer(value) => binding_integer_text(&nodes, index, value),
        })
    }

    #[doc(hidden)]
    pub fn bundle_text_node_string(
        &mut self,
        nodes: &Rc<JsValue>,
        index: u32,
        read: impl Fn() -> String + 'static,
    ) -> Result<(), JsValue> {
        let nodes = Rc::clone(nodes);
        self.bind_dom_infallible(move || binding_text(&nodes, index, &read()))
    }

    #[doc(hidden)]
    pub fn bundle_attr(
        &mut self,
        nodes: &Rc<JsValue>,
        index: u32,
        name: &'static str,
        read: impl Fn() -> Option<String> + 'static,
    ) -> Result<(), JsValue> {
        let nodes = Rc::clone(nodes);
        let name = strings::static_attribute(name);
        self.bind_dom(move || match read() {
            Some(value) => binding_set_attribute(&nodes, index, &name, &value),
            None => binding_remove_attribute(&nodes, index, &name),
        })
    }

    /// An attribute that is always present, with the text path's conversion.
    #[doc(hidden)]
    pub fn bundle_attr_value(
        &mut self,
        nodes: &Rc<JsValue>,
        index: u32,
        name: &'static str,
        read: impl Fn() -> crate::dom::text_value::Output + 'static,
    ) -> Result<(), JsValue> {
        use crate::dom::text_value::Output;
        let nodes = Rc::clone(nodes);
        let name = strings::static_attribute(name);
        self.bind_dom(move || match read() {
            Output::String(value) => binding_set_attribute(&nodes, index, &name, &value),
            Output::Integer(value) => binding_set_integer_attribute(&nodes, index, &name, value),
        })
    }

    #[doc(hidden)]
    pub fn bundle_on(
        &mut self,
        nodes: &Rc<JsValue>,
        index: u32,
        event: &str,
        handler: impl FnMut(Event) + 'static,
    ) -> Result<(), JsValue> {
        self.on_bundle(nodes, index, event, handler)
    }
}
