//! Sibling ranges delimited by two comment anchors, and attaching views to them.
use super::{JsValue, Scope};
use wasm_bindgen::JsCast;
use web_sys::{Element, Node};

/// A handle to a range of sibling nodes between two comment anchors.
///
/// A range is either an insertion slot, which a compiled template or
/// [`Scope::mount_point`] creates and whose views are replaced over time, or a
/// fragment scope's own content, anchors included. Cloning copies the handle;
/// whoever created the anchors owns them.
#[derive(Clone)]
pub struct MountPoint {
    pub(super) start: Node,
    pub(super) end: Node,
}

/// Owns the anchors of a range from [`MountPoint::append`]. Dropping it removes
/// them, but not the nodes of views still inside them.
#[must_use = "dropping the anchors removes the range"]
pub struct Anchors(MountPoint);

impl Drop for Anchors {
    fn drop(&mut self) {
        for node in [&self.0.start, &self.0.end] {
            if let Some(parent) = node.parent_node() {
                if let Err(error) = parent.remove_child(node) {
                    web_sys::console::error_1(&error);
                }
            }
        }
    }
}

impl MountPoint {
    /// Append an empty range to `container`, owned by the returned anchors.
    pub fn append(container: &Element) -> Result<(Self, Anchors), JsValue> {
        let document = super::document()?;
        let start: Node = document.create_comment("fusor:mount").into();
        let end: Node = document.create_comment("fusor:end").into();
        container.append_child(&start)?;
        if let Err(error) = container.append_child(&end) {
            if let Err(rollback) = container.remove_child(&start) {
                web_sys::console::error_1(&rollback);
            }
            return Err(error);
        }
        let point = Self { start, end };
        Ok((point.clone(), Anchors(point)))
    }

    /// Native container of this range.
    pub fn parent_element(&self) -> Result<Element, JsValue> {
        self.validate()?;
        self.end
            .parent_element()
            .ok_or_else(|| JsValue::from_str("mount range has no element parent"))
    }

    /// Check that the anchors are siblings in order, and return their parent.
    pub(super) fn validate(&self) -> Result<Node, JsValue> {
        self.walk(|_| {})?;
        Ok(self
            .start
            .parent_node()
            .expect("sibling anchors have a parent"))
    }

    /// Visit the nodes strictly between the anchors, failing unless the
    /// anchors are siblings in order.
    fn walk(&self, mut visit: impl FnMut(Node)) -> Result<(), JsValue> {
        let mut cursor = self.start.next_sibling();
        while let Some(node) = cursor {
            if node.is_same_node(Some(&self.end)) {
                return Ok(());
            }
            cursor = node.next_sibling();
            visit(node);
        }
        let (start, end) = (self.start.parent_node(), self.end.parent_node());
        Err(JsValue::from_str(match (start, end) {
            (None, _) => "detached mount range",
            (Some(start), Some(end)) if start.is_same_node(Some(&end)) => {
                "mount range anchors are out of order"
            }
            _ => "mount range anchors have different parents",
        }))
    }

    pub(super) fn is_empty(&self) -> Result<bool, JsValue> {
        let mut empty = true;
        self.walk(|_| empty = false)?;
        Ok(empty)
    }

    /// A present server child is one element or a complete fragment range;
    /// an absent child must leave the slot empty.
    pub(super) fn hydrated_root(
        &self,
        present: bool,
    ) -> Result<Option<super::hydration::Target>, JsValue> {
        let mut inner = Vec::new();
        self.walk(|node| inner.push(node))?;
        match (present, inner.as_slice()) {
            (false, []) => Ok(None),
            (true, [root]) if root.node_type() == Node::ELEMENT_NODE => Ok(Some(
                super::hydration::Target::Root(root.clone().unchecked_into()),
            )),
            (true, _) => self
                .fragment()
                .map(super::hydration::Target::Range)
                .map(Some),
            _ => Err(JsValue::from_str(
                "server component identity/shape mismatch",
            )),
        }
    }

    pub(super) fn fragment(&self) -> Result<Self, JsValue> {
        self.validate()?;
        let start = self
            .start
            .next_sibling()
            .ok_or_else(|| JsValue::from_str("missing fragment start"))?;
        let range = Self::from_start(start)?;
        if !range
            .end
            .next_sibling()
            .is_some_and(|node| node.is_same_node(Some(&self.end)))
        {
            return Err(JsValue::from_str(
                "server component identity/shape mismatch",
            ));
        }
        Ok(range)
    }

    pub(super) fn first_element(&self) -> Result<Element, JsValue> {
        let mut first = None;
        self.walk(|node| {
            if first.is_none() && node.node_type() == Node::ELEMENT_NODE {
                first = Some(node.unchecked_into());
            }
        })?;
        first.ok_or_else(|| JsValue::from_str("fragment row requires a native element"))
    }

    pub(super) fn from_start(start: Node) -> Result<Self, JsValue> {
        let marker = start
            .node_value()
            .filter(|value| {
                start.node_type() == Node::COMMENT_NODE
                    && value.starts_with(crate::template::FRAGMENT_START)
            })
            .ok_or_else(|| JsValue::from_str("missing fragment start"))?;
        let mut markers = vec![marker];
        let mut cursor = start.next_sibling();
        while let Some(node) = cursor {
            cursor = node.next_sibling();
            if node.node_type() != Node::COMMENT_NODE {
                continue;
            }
            let value = node.node_value().unwrap_or_default();
            if value.starts_with(crate::template::FRAGMENT_START) {
                markers.push(value);
                continue;
            }
            if !value.starts_with(crate::template::FRAGMENT_END) {
                continue;
            }
            if markers.pop().as_deref() != value.strip_prefix('/') {
                return Err(JsValue::from_str("mismatched fragment anchors"));
            }
            if markers.is_empty() {
                return Ok(Self { start, end: node });
            }
        }
        Err(JsValue::from_str("missing or unpaired fragment anchors"))
    }

    /// Both anchors and everything between them.
    pub(super) fn nodes(&self) -> Result<Vec<Node>, JsValue> {
        let mut nodes = vec![self.start.clone()];
        self.walk(|node| nodes.push(node))?;
        nodes.push(self.end.clone());
        Ok(nodes)
    }

    pub(super) fn contains(&self, target: &Node) -> bool {
        self.nodes()
            .is_ok_and(|nodes| nodes.iter().any(|node| node.contains(Some(target))))
    }

    /// Whether `node` is already the last node of the range.
    pub(super) fn precedes_end(&self, node: &Node) -> bool {
        node.next_sibling()
            .is_some_and(|next| next.is_same_node(Some(&self.end)))
    }

    /// Remove this fragment's anchors and content from the document.
    pub(super) fn remove(&self) {
        // Structural publication removes retired ranges before their scopes drop.
        if self.start.parent_node().is_none() && self.end.parent_node().is_none() {
            return;
        }
        let nodes = match self.nodes() {
            Ok(nodes) => nodes,
            Err(error) => {
                web_sys::console::error_1(&error);
                return;
            }
        };
        for node in nodes {
            #[cfg(feature = "islands")]
            if let Some(element) = node.dyn_ref::<Element>() {
                super::delivery::dispose_tree(element);
            }
            if let Some(parent) = node.parent_node() {
                if let Err(error) = parent.remove_child(&node) {
                    web_sys::console::error_1(&error);
                }
            }
        }
    }
}

impl Scope {
    /// Whether this view is still in its detached preparation storage.
    pub fn is_detached(&self) -> bool {
        self.root.parent_node().is_none()
            && self.fragment.as_ref().is_none_or(|range| {
                range
                    .validate()
                    .is_ok_and(|parent| parent.is_same_node(Some(&self.root)))
            })
    }

    pub(super) fn validate_nodes(&self) -> Result<(), JsValue> {
        if let Some(range) = &self.fragment {
            range.validate()?;
        }
        Ok(())
    }

    pub(super) fn first_node(&self) -> &Node {
        self.fragment
            .as_ref()
            .map_or(self.root.as_ref(), |range| &range.start)
    }

    pub(super) fn last_node(&self) -> &Node {
        self.fragment
            .as_ref()
            .map_or(self.root.as_ref(), |range| &range.end)
    }

    pub(super) fn insert_before(
        &self,
        parent: &Element,
        anchor: Option<&Node>,
    ) -> Result<(), JsValue> {
        if let Some(range) = &self.fragment {
            for node in range.nodes()? {
                super::strings::insert_before(parent, &node, anchor)?;
            }
        } else {
            super::strings::insert_before(parent, &self.root, anchor)?;
        }
        Ok(())
    }

    pub(super) fn remove_nodes(&self) {
        if let Some(range) = &self.fragment {
            range.remove();
        } else {
            super::remove_tree(&self.root);
        }
    }

    /// Append an empty insertion range to a container. Dropping this scope
    /// removes the anchors, but not the nodes of views still inside them:
    /// callers retain and dispose any child views they mount there.
    pub fn mount_point(&mut self, container: &Element) -> Result<MountPoint, JsValue> {
        let (point, anchors) = MountPoint::append(container)?;
        self.retain(anchors);
        Ok(point)
    }

    /// Insert this prepared view at the end of `target` without activating
    /// it. A root view is removed when the scope drops; a fragment removes its
    /// own range. Retain the scope and finish preparation before committing.
    pub fn attach_at(&mut self, target: &MountPoint) -> Result<(), JsValue> {
        self.insert_before(&target.parent_element()?, Some(&target.end))?;
        self.remove_on_drop = true;
        Ok(())
    }

    /// Move this fragment scope's range to the end of `target`.
    pub(super) fn attach_fragment(&self, target: &MountPoint) -> Result<(), JsValue> {
        if self.fragment.is_none() {
            return Err(JsValue::from_str("expected a fragment scope"));
        }
        self.insert_before(&target.parent_element()?, Some(&target.end))
    }
}
