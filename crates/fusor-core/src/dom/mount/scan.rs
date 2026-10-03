use super::{
    ElementHandle, Handles, Mounts, Resolution, Slot, TextPosition, element_text, invalid,
    text_slot,
};
use crate::dom::{MountPoint, Scope, document, is_html, strings};
use crate::template::{
    self, ChildPolicy, ElementId, MountId, MountMarker, TemplateDescriptor, TextId, TextMarker,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Display,
};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, Node};

pub(super) fn resolve(
    descriptor: &TemplateDescriptor,
    expected_mounts: &'static [MountId],
    scope: &Scope,
) -> Result<Resolution, JsValue> {
    let mut found = scan(descriptor, scope)?;
    let handles = found.elements(descriptor)?;
    let mut slots = found.anchored_texts(descriptor)?;
    found.element_texts(descriptor, &handles, &mut slots)?;
    let mounts = found.mounts(expected_mounts, scope)?;
    Ok((handles, slots, mounts))
}

#[derive(Default)]
struct ScannedNodes {
    elements: BTreeMap<ElementId, Element>,
    starts: BTreeMap<TextId, Node>,
    ends: BTreeMap<TextId, Node>,
    text_elements: BTreeMap<TextId, Element>,
    mount_starts: BTreeMap<MountId, Node>,
    mount_ends: BTreeMap<MountId, Node>,
    skip_mount: Option<MountId>,
}

fn scan(descriptor: &TemplateDescriptor, scope: &Scope) -> Result<ScannedNodes, JsValue> {
    let document = document()?;
    let root = scope.root();
    let fragment = scope.fragment.as_ref();
    let mut found = ScannedNodes::default();
    let managed: BTreeSet<_> = descriptor
        .elements
        .iter()
        .filter(|element| element.children == ChildPolicy::Managed)
        .map(|element| element.id)
        .collect();
    // SHOW_ALL is the DOM API's default. Include the root, then visit its
    // descendants once; inert template contents are separate DOM trees.
    let traversal_root: Node = fragment
        .and_then(|point| point.start.parent_node())
        .unwrap_or_else(|| root.clone().into());
    let walker = document.create_tree_walker(&traversal_root)?;
    let mut current: Option<Node> = if let Some(point) = fragment {
        walker.set_current_node(&point.start);
        walker.next_node()?
    } else {
        Some(root.clone().into())
    };
    while let Some(node) = current {
        if fragment.is_some_and(|point| node.is_same_node(Some(&point.end))) {
            break;
        }
        // A server-rendered child resolves its own descriptor. Skip whole
        // sibling subtrees, including any nested component anchor pairs.
        if let Some(id) = found.skip_mount {
            if node.node_type() == Node::COMMENT_NODE
                && MountMarker::parse(&node.node_value().unwrap_or_default()).map_err(invalid)?
                    == Some(MountMarker::End(id))
            {
                found.skip_mount = None;
            } else {
                current = walker.next_sibling()?;
                continue;
            }
        }
        let skip_children = found.visit(node, &managed, scope)?;
        current = if skip_children {
            next_after_subtree(&walker)?
        } else {
            walker.next_node()?
        };
    }

    Ok(found)
}

impl ScannedNodes {
    fn visit(
        &mut self,
        node: Node,
        managed: &BTreeSet<ElementId>,
        scope: &Scope,
    ) -> Result<bool, JsValue> {
        let mut skip_children = false;
        match node.node_type() {
            Node::ELEMENT_NODE => {
                let element: Element = node.unchecked_into();
                if let Some(value) = element.get_attribute(template::TEXT_ELEMENT_ATTRIBUTE) {
                    let id: TextId = value.parse().map_err(invalid)?;
                    insert(&mut self.text_elements, id, element.clone(), "text element")?;
                }
                if let Some(value) = strings::attribute(&element, strings::Name::Element) {
                    let id: ElementId = value.parse().map_err(invalid)?;
                    skip_children = managed.contains(&id);
                    insert(&mut self.elements, id, element, "element")?;
                }
            }
            Node::COMMENT_NODE => self.comment(node, scope)?,
            _ => {}
        }
        Ok(skip_children)
    }

    fn comment(&mut self, node: Node, scope: &Scope) -> Result<(), JsValue> {
        let value = node.node_value().unwrap_or_default();
        if let Some(marker) = MountMarker::parse(&value).map_err(invalid)? {
            let (id, anchors) = match marker {
                MountMarker::Start(id) => {
                    if scope.is_hydrating() {
                        self.skip_mount = Some(id);
                    }
                    (id, &mut self.mount_starts)
                }
                MountMarker::End(id) => (id, &mut self.mount_ends),
            };
            insert(anchors, id, node, "component anchor")?;
        } else if let Some(marker) = TextMarker::parse(&value).map_err(invalid)? {
            let (id, anchors, kind) = match marker {
                TextMarker::Start(id) => (id, &mut self.starts, "text start"),
                TextMarker::End(id) => (id, &mut self.ends, "text end"),
            };
            insert(anchors, id, node, kind)?;
        }
        Ok(())
    }

    fn elements(&mut self, descriptor: &TemplateDescriptor) -> Result<Handles, JsValue> {
        let mut handles = Handles::new();
        for expected in descriptor.elements {
            let element = take(&mut self.elements, expected.id, "element")?;
            if element.local_name() != expected.tag || !is_html(&element) {
                return Err(invalid(format_args!(
                    "element {} must be <{}>, found <{}>",
                    expected.id,
                    expected.tag,
                    element.local_name()
                )));
            }
            let handle = if expected.tag == "input" {
                ElementHandle::Input(
                    element
                        .dyn_into()
                        .map_err(|_| invalid("expected an HTML input"))?,
                )
            } else {
                ElementHandle::Element(element)
            };
            handles.insert(expected.id, handle);
        }
        if !self.elements.is_empty() {
            return Err(invalid("unexpected element identifiers"));
        }
        handles.finish();

        Ok(handles)
    }

    fn anchored_texts(&mut self, descriptor: &TemplateDescriptor) -> Result<Vec<Slot>, JsValue> {
        // Validate every pair before adding text nodes or subscribing effects.
        let mut slots = Vec::new();
        for id in descriptor.texts {
            let start = take(&mut self.starts, *id, "text start")?;
            let end = take(&mut self.ends, *id, "text end")?;
            let text = text_slot(*id, &start, &end)?;
            slots.push(Slot {
                id: *id,
                position: TextPosition::Anchored { start, end },
                existing: text,
            });
        }
        if !self.starts.is_empty() || !self.ends.is_empty() {
            return Err(invalid("unexpected text identifiers"));
        }
        Ok(slots)
    }

    fn element_texts(
        &mut self,
        descriptor: &TemplateDescriptor,
        handles: &Handles,
        slots: &mut Vec<Slot>,
    ) -> Result<(), JsValue> {
        for expected in descriptor.text_elements {
            let id = expected.id;
            let element = take(&mut self.text_elements, id, "or mismatched text element")?;
            if element.local_name() != expected.tag || !is_html(&element) {
                return Err(invalid(format_args!(
                    "text element {id} must be <{}>",
                    expected.tag
                )));
            }
            if let Some(host) = expected.host {
                if !matches!(handles.get(&host), Some(ElementHandle::Element(bound)) if element.is_same_node(Some(bound)))
                {
                    return Err(invalid(format_args!("mismatched text host {host}")));
                }
            }
            let existing = element_text(id, &element)?;
            slots.push(Slot {
                id,
                position: TextPosition::Element(element),
                existing,
            });
        }
        if !self.text_elements.is_empty() {
            return Err(invalid("unexpected text elements"));
        }
        Ok(())
    }

    fn mounts(&mut self, expected_mounts: &[MountId], scope: &Scope) -> Result<Mounts, JsValue> {
        let mut mounts = Mounts::new();
        for id in expected_mounts {
            let point = MountPoint {
                start: take(&mut self.mount_starts, *id, "component start")?,
                end: take(&mut self.mount_ends, *id, "component end")?,
            };
            if scope.is_hydrating() {
                point.validate()?;
            } else if !point
                .start
                .next_sibling()
                .is_some_and(|next| next.is_same_node(Some(&point.end)))
            {
                return Err(invalid(format_args!(
                    "component mount {id} must initially be empty and paired"
                )));
            }
            mounts.insert(*id, point);
        }
        if !self.mount_starts.is_empty() || !self.mount_ends.is_empty() {
            return Err(invalid("unexpected component anchors"));
        }
        Ok(mounts)
    }
}

fn insert<K: Ord + Display + Copy, V>(
    found: &mut BTreeMap<K, V>,
    id: K,
    value: V,
    what: &str,
) -> Result<(), JsValue> {
    if found.insert(id, value).is_some() {
        return Err(invalid(format_args!("duplicate {what} {id}")));
    }
    Ok(())
}

fn take<K: Ord + Display, V>(found: &mut BTreeMap<K, V>, id: K, what: &str) -> Result<V, JsValue> {
    found
        .remove(&id)
        .ok_or_else(|| invalid(format_args!("missing {what} {id}")))
}

fn next_after_subtree(walker: &web_sys::TreeWalker) -> Result<Option<Node>, JsValue> {
    loop {
        if let Some(sibling) = walker.next_sibling()? {
            return Ok(Some(sibling));
        }
        if walker.parent_node()?.is_none() {
            return Ok(None);
        }
    }
}
