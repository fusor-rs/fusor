//! Bounded certificates for exact copies of previously validated template DOM.
//! Snapshots live in the inert template document, never an application scope.
use super::{
    ElementHandle, Handles, MountPoint, Mounts, Resolution, Slot, TextPosition, element_text,
    invalid, text_slot,
};
use crate::template::{
    ComponentId, ElementDescriptor, ElementId, MountId, TemplateDescriptor, TextElementDescriptor,
    TextId,
};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, HtmlTemplateElement, Node};

type Path = Vec<u32>;
struct Plan {
    component: ComponentId,
    version: u32,
    elements: &'static [ElementDescriptor],
    texts: &'static [TextId],
    text_elements: &'static [TextElementDescriptor],
    mounts: &'static [MountId],
    pristine: Node,
    handles: Vec<(ElementId, Path, bool)>,
    slots: Vec<(TextId, SlotPath)>,
    points: Vec<(MountId, Path, Path)>,
}
enum SlotPath {
    Anchored(Path, Path),
    Element(Path),
}
impl Plan {
    fn matches(&self, descriptor: &TemplateDescriptor, mounts: &'static [MountId]) -> bool {
        self.component == descriptor.component
            && self.version == descriptor.version
            // Static slices are immutable. Pointer+length identity is a stricter
            // cache key than semantic equality, including for nonstatic `self`.
            && std::ptr::eq(self.elements, descriptor.elements)
            && std::ptr::eq(self.texts, descriptor.texts)
            && std::ptr::eq(self.text_elements, descriptor.text_elements)
            && std::ptr::eq(self.mounts, mounts)
    }
}
thread_local! {
    static PLANS: RefCell<VecDeque<Rc<Plan>>> = const { RefCell::new(VecDeque::new()) };
}

fn path(root: &Node, target: &Node) -> Result<Path, JsValue> {
    let mut path = Vec::new();
    let mut node = target.clone();
    while !node.is_same_node(Some(root)) {
        let mut sibling = node.previous_sibling();
        let mut index = 0;
        while let Some(previous) = sibling {
            index += 1;
            sibling = previous.previous_sibling();
        }
        path.push(index);
        node = node
            .parent_node()
            .ok_or_else(|| invalid("detached cached handle"))?;
    }
    path.reverse();
    Ok(path)
}
fn follow(root: &Node, path: &Path) -> Result<Node, JsValue> {
    let mut node = root.clone();
    for &index in path {
        node = node
            .child_nodes()
            .item(index)
            .ok_or_else(|| invalid("template changed during resolution"))?;
    }
    Ok(node)
}

pub(super) fn resolve(
    descriptor: &TemplateDescriptor,
    mounts: &'static [MountId],
    root: &Element,
) -> Result<Option<Resolution>, JsValue> {
    // Release the Rust registry borrow before calling native DOM methods.
    let plan = PLANS.with(|plans| {
        plans
            .borrow()
            .iter()
            .rev()
            .find(|plan| plan.matches(descriptor, mounts))
            .cloned()
    });
    let Some(plan) = plan.filter(|plan| root.is_equal_node(Some(&plan.pristine))) else {
        return Ok(None);
    };
    let mut handles = Handles::new();
    for (id, path, input) in &plan.handles {
        let node = follow(root, path)?;
        let handle = if *input {
            ElementHandle::Input(node.dyn_into()?)
        } else {
            ElementHandle::Element(node.dyn_into()?)
        };
        handles.insert(*id, handle);
    }
    // Resolve every path before inserting any missing text nodes. Inserting the
    // first slot would otherwise shift sibling paths for later slots/handles.
    let mut slots = Vec::new();
    for (id, location) in &plan.slots {
        let (position, existing) = match location {
            SlotPath::Anchored(start, end) => {
                let start = follow(root, start)?;
                let end = follow(root, end)?;
                let existing = text_slot(*id, &start, &end)?;
                (TextPosition::Anchored { start, end }, existing)
            }
            SlotPath::Element(host) => {
                let element: Element = follow(root, host)?.dyn_into()?;
                let existing = element_text(*id, &element)?;
                (TextPosition::Element(element), existing)
            }
        };
        slots.push(Slot {
            id: *id,
            position,
            existing,
        });
    }
    let mut mounts = Mounts::new();
    for (id, start, end) in &plan.points {
        mounts.insert(
            *id,
            MountPoint {
                start: follow(root, start)?,
                end: follow(root, end)?,
            },
        );
    }
    handles.finish();
    Ok(Some((handles, slots, mounts)))
}

pub(super) fn remember(
    descriptor: &TemplateDescriptor,
    mounts: &'static [MountId],
    root: &Element,
    resolution: &Resolution,
) -> Result<(), JsValue> {
    let (handles, slots, points) = resolution;
    let document = root
        .owner_document()
        .ok_or_else(|| invalid("template has no document"))?;
    let template: HtmlTemplateElement = document.create_element("template")?.dyn_into()?;
    let inert = template
        .content()
        .owner_document()
        .ok_or_else(|| invalid("template has no inert document"))?;
    // Import into the inert document: custom elements must not be constructed a
    // second time, and the certificate must not activate resource-loading nodes.
    let pristine = inert.import_node_with_deep(root, true)?;
    let handles = handles
        .iter()
        .map(|(id, handle)| {
            let (node, input): (&Node, bool) = match handle {
                ElementHandle::Element(element) => (element.as_ref(), false),
                ElementHandle::Input(input) => (input.as_ref(), true),
            };
            Ok((*id, path(root, node)?, input))
        })
        .collect::<Result<_, JsValue>>()?;
    let slots = slots
        .iter()
        .map(|slot| {
            let location = match &slot.position {
                TextPosition::Anchored { start, end } => {
                    SlotPath::Anchored(path(root, start)?, path(root, end)?)
                }
                TextPosition::Element(element) => SlotPath::Element(path(root, element)?),
            };
            Ok((slot.id, location))
        })
        .collect::<Result<_, JsValue>>()?;
    let points = points
        .iter()
        .map(|(id, point)| Ok((*id, path(root, &point.start)?, path(root, &point.end)?)))
        .collect::<Result<_, JsValue>>()?;
    let plan = Rc::new(Plan {
        component: descriptor.component,
        version: descriptor.version,
        elements: descriptor.elements,
        texts: descriptor.texts,
        text_elements: descriptor.text_elements,
        mounts,
        pristine,
        handles,
        slots,
        points,
    });
    PLANS.with(|plans| {
        let mut plans = plans.borrow_mut();
        plans.retain(|previous| !previous.matches(descriptor, mounts));
        if plans.len() >= 32 {
            plans.pop_front();
        }
        plans.push_back(plan);
    });
    Ok(())
}
