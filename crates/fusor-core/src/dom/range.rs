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
                let _ = parent.remove_child(node);
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
            let _ = container.remove_child(&start);
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

    /// The server-rendered root in this slot when hydration begins. There must
    /// be exactly one element when a child is `present`, and nothing otherwise.
    pub(super) fn hydrated_root(&self, present: bool) -> Result<Option<Element>, JsValue> {
        let mut inner = Vec::new();
        self.walk(|node| inner.push(node))?;
        match (present, inner.as_slice()) {
            (false, []) => Ok(None),
            (true, [root]) if root.node_type() == Node::ELEMENT_NODE => {
                Ok(Some(root.clone().unchecked_into()))
            }
            _ => Err(JsValue::from_str(
                "server component identity/shape mismatch",
            )),
        }
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

    /// Insert `node` last, without validating the range.
    pub(super) fn push(&self, node: &Node) -> Result<(), JsValue> {
        self.end
            .parent_node()
            .ok_or_else(|| JsValue::from_str("detached mount range"))?
            .insert_before(node, Some(&self.end))?;
        Ok(())
    }

    /// Remove this fragment's anchors and content from the document.
    pub(super) fn remove(&self) {
        let Ok(nodes) = self.nodes() else {
            return;
        };
        for node in nodes {
            #[cfg(feature = "islands")]
            if let Some(element) = node.dyn_ref::<Element>() {
                super::delivery::dispose_tree(element);
            }
            if let Some(parent) = node.parent_node() {
                let _ = parent.remove_child(&node);
            }
        }
    }
}

impl Scope {
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
        if self.fragment.is_some() {
            return self.attach_fragment(target);
        }
        target
            .validate()?
            .insert_before(&self.root, Some(&target.end))?;
        self.remove_on_drop = true;
        Ok(())
    }

    /// Move this fragment scope's range to the end of `target`.
    pub(super) fn attach_fragment(&self, target: &MountPoint) -> Result<(), JsValue> {
        let parent = target.validate()?;
        let fragment = self
            .fragment
            .as_ref()
            .ok_or_else(|| JsValue::from_str("expected a children fragment"))?;
        for node in fragment.nodes()? {
            parent.insert_before(&node, Some(&target.end))?;
        }
        Ok(())
    }
}
