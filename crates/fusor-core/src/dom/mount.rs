//! Validate the serialized template once and resolve native DOM handles.

use super::{JsValue, MountPoint, Scope, document, strings};
use crate::OwnerHandle;
use crate::template::{
    self, ChildPolicy, ElementId, MountId, RootKind, TemplateDescriptor, TextId,
};
use std::{cell::Cell, collections::BTreeMap, rc::Rc};
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlInputElement, HtmlTemplateElement, Node, Text};

mod cache;
mod flat;
mod scan;

// Collect sparse IDs without shifting existing entries, then finalize before
// lookup. Generated code can move handles out of the compact sorted storage.
struct Nodes<K, V>(Vec<(K, Option<V>)>);
impl<K: Ord, V> Nodes<K, V> {
    fn new() -> Self {
        Self(Vec::new())
    }
    fn insert(&mut self, id: K, value: V) {
        self.0.push((id, Some(value)));
    }
    fn finish(&mut self) {
        // Most element descriptors are already ordered. Text descriptors can
        // interleave anchored and direct slots, which arrive in separate groups.
        if self.0.windows(2).all(|pair| pair[0].0 < pair[1].0) {
            return;
        }
        // Stable sorting retains insertion order among equal IDs. Keep the
        // first key and last value, as BTreeMap::insert did before compaction.
        self.0.sort_by(|left, right| left.0.cmp(&right.0));
        self.0.dedup_by(|next, previous| {
            if next.0 == previous.0 {
                previous.1 = next.1.take();
                true
            } else {
                false
            }
        });
    }
    fn index(&self, id: &K) -> Option<usize> {
        self.0.binary_search_by(|(key, _)| key.cmp(id)).ok()
    }
    fn get(&self, id: &K) -> Option<&V> {
        self.index(id).and_then(|index| self.0[index].1.as_ref())
    }
    fn take(&mut self, id: &K) -> Option<V> {
        self.index(id).and_then(|index| self.0[index].1.take())
    }
    fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.0
            .iter()
            .filter_map(|(id, value)| value.as_ref().map(|value| (id, value)))
    }
}
type Handles = Nodes<ElementId, ElementHandle>;
struct Slot {
    id: TextId,
    position: TextPosition,
    existing: Option<Text>,
}
enum TextPosition {
    Anchored { start: Node, end: Node },
    Element(Element),
}
type Mounts = BTreeMap<MountId, MountPoint>;
type Resolution = (Handles, Vec<Slot>, Mounts);
type Mounted = (Scope, TemplateNodes);

enum ElementHandle {
    Element(Element),
    Input(HtmlInputElement),
}

/// All handles have been checked against the compiler's template descriptor.
pub struct TemplateNodes {
    binding_bundle: Option<Rc<JsValue>>,
    elements: Handles,
    texts: Nodes<TextId, Text>,
    mounts: Mounts,
}

/// Deepest synchronous chain of generated component mounts.
const MAX_NESTING: usize = 128;

thread_local! {
    static NESTING: Cell<usize> = const { Cell::new(0) };
}

/// Held by a generated prepare while it mounts. Bounds recursive component
/// tags, including cycles across HTML modules.
#[doc(hidden)]
pub struct NestingGuard;

impl NestingGuard {
    pub fn enter() -> Result<Self, JsValue> {
        let depth = NESTING.get();
        if depth >= MAX_NESTING {
            return Err(JsValue::from_str(&format!(
                "fusor: component nesting exceeds {MAX_NESTING}; check for recursive component tags"
            )));
        }
        NESTING.set(depth + 1);
        Ok(Self)
    }
}

impl Drop for NestingGuard {
    fn drop(&mut self) {
        NESTING.set(NESTING.get() - 1);
    }
}

fn invalid(message: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&format!("fusor: template mismatch: {message}"))
}

impl TemplateNodes {
    fn bundle(binding_bundle: JsValue) -> Self {
        Self {
            binding_bundle: Some(Rc::new(binding_bundle)),
            elements: Handles::new(),
            texts: Nodes::new(),
            mounts: Mounts::new(),
        }
    }
}

/// Generated code moves each validated handle out exactly once.
#[doc(hidden)]
impl TemplateNodes {
    /// A generated flat component can retain native targets without unpacking
    /// every handle across the Wasm boundary.
    pub fn take_binding_bundle(&mut self) -> Option<Rc<JsValue>> {
        self.binding_bundle.take()
    }

    pub fn take_element(&mut self, id: ElementId) -> Result<Element, JsValue> {
        match self.elements.take(&id) {
            Some(ElementHandle::Element(element)) => Ok(element),
            Some(ElementHandle::Input(input)) => Ok(input.into()),
            None => Err(invalid(format_args!("missing element {id}"))),
        }
    }

    pub fn take_input(&mut self, id: ElementId) -> Result<HtmlInputElement, JsValue> {
        match self.elements.take(&id) {
            Some(ElementHandle::Input(input)) => Ok(input),
            _ => Err(invalid(format_args!("element {id} is not an HTML input"))),
        }
    }

    pub fn take_text(&mut self, id: TextId) -> Result<Text, JsValue> {
        self.texts
            .take(&id)
            .ok_or_else(|| invalid(format_args!("missing text {id}")))
    }

    pub fn take_mount_point(&mut self, id: MountId) -> Result<MountPoint, JsValue> {
        self.mounts
            .remove(&id)
            .ok_or_else(|| invalid(format_args!("missing component mount {id}")))
    }
}

#[derive(Clone, Copy)]
enum MountMode<'a> {
    Active,
    Prepared(Option<&'a OwnerHandle>),
    Bundled(Option<&'a OwnerHandle>),
}

impl MountMode<'_> {
    fn bundled(self) -> bool {
        matches!(self, Self::Bundled(_))
    }

    fn scope(self, root: Element) -> Scope {
        match self {
            Self::Active => Scope::new(root),
            Self::Prepared(parent) | Self::Bundled(parent) => Scope::new_prepared(root, parent),
        }
    }

    fn clone_template(self, template: &HtmlTemplateElement) -> Result<Scope, JsValue> {
        Ok(self.scope(Scope::clone_template_root(template)?))
    }
}

fn finish_preparation(
    mounted: Result<Mounted, JsValue>,
    parent: Option<&OwnerHandle>,
) -> Result<Mounted, JsValue> {
    let (mut scope, nodes) = mounted?;
    scope.finish_owner_preparation(parent);
    Ok((scope, nodes))
}

/// The first element of compiler-embedded HTML, parsed in a detached template.
fn parse_html(html: &str) -> Result<Option<Element>, JsValue> {
    let wrapper = document()?
        .create_element("template")?
        .dyn_into::<HtmlTemplateElement>()?;
    wrapper.set_inner_html(html);
    Ok(wrapper.content().first_element_child())
}

impl TemplateDescriptor {
    /// Compiler entry point: retain the final prepared owner and readiness token.
    /// Integration setup still follows successful native descriptor validation.
    #[doc(hidden)]
    pub fn prepare_with_points(
        &self,
        html: &'static str,
        mounts: &'static [MountId],
        parent: Option<&OwnerHandle>,
    ) -> Result<Mounted, JsValue> {
        let mounted = self.mount_root(html, mounts, MountMode::Prepared(parent), false);
        finish_preparation(mounted, parent)
    }

    /// Generated ordinary flat bindings retain the validated native bundle.
    /// Coherent preparation keeps the existing typed patch targets.
    #[doc(hidden)]
    pub fn prepare_with_binding_bundle(
        &self,
        html: &'static str,
        parent: Option<&OwnerHandle>,
    ) -> Result<Mounted, JsValue> {
        let mode = if super::coherent::parent_is_coherent(parent) {
            MountMode::Prepared(parent)
        } else {
            MountMode::Bundled(parent)
        };
        finish_preparation(self.mount_root(html, &[], mode, false), parent)
    }

    /// Compiler entry point for a template library without its own browser document.
    /// Package-local component IDs are not looked up in the consuming document.
    #[doc(hidden)]
    pub fn prepare_embedded(
        &self,
        html: &'static str,
        mounts: &'static [MountId],
        parent: Option<&OwnerHandle>,
        bundled: bool,
    ) -> Result<Mounted, JsValue> {
        let mode = if bundled && !super::coherent::parent_is_coherent(parent) {
            MountMode::Bundled(parent)
        } else {
            MountMode::Prepared(parent)
        };
        finish_preparation(self.mount_root(html, mounts, mode, true), parent)
    }

    /// Resolve a wrapper-free child group, adopting an existing native range
    /// during hydration without moving or replacing its nodes.
    #[doc(hidden)]
    pub fn prepare_fragment(
        &self,
        html: &'static str,
        mounts: &'static [MountId],
        parent: Option<&OwnerHandle>,
    ) -> Result<Mounted, JsValue> {
        let mounted = self.mount_fragment(html, mounts, MountMode::Prepared(parent));
        finish_preparation(mounted, parent)
    }

    pub fn mount(&self) -> Result<Mounted, JsValue> {
        self.mount_document_root(&[], MountMode::Active)
    }

    fn mount_root(
        &self,
        html: &'static str,
        mounts: &'static [MountId],
        mode: MountMode<'_>,
        embedded: bool,
    ) -> Result<Mounted, JsValue> {
        #[cfg(feature = "islands")]
        {
            if let Some(root) = super::hydration::take_root() {
                if mode.bundled() && mounts.is_empty() && self.is_flat() {
                    // The same identity checks and resolution as below, in one
                    // native call.
                    let binding_bundle = flat::hydrate_root(self, &root)?;
                    let mut scope = mode.scope(root);
                    scope.hydrating = true;
                    return Ok((scope, TemplateNodes::bundle(binding_bundle)));
                }
                let metadata = strings::descriptor(self.component, self.version);
                if !metadata.version_matches(&root) || !metadata.component_matches(&root) {
                    return Err(invalid(
                        "server root identity differs from the browser template",
                    ));
                }
                let mut scope = mode.scope(root);
                scope.hydrating = true;
                return self.resolve(scope, mounts, mode.bundled());
            }
        }
        #[cfg(feature = "islands")]
        let embedded = embedded || super::delivery::enabled();
        if embedded {
            let root = parse_html(html)?.ok_or_else(|| invalid("empty embedded template"))?;
            let scope = match self.kind {
                RootKind::Template => mode.clone_template(
                    &root
                        .dyn_into::<HtmlTemplateElement>()
                        .map_err(|_| invalid("expected embedded HTML template"))?,
                )?,
                RootKind::Existing => mode.scope(root),
            };
            return self.resolve(scope, mounts, mode.bundled());
        }
        self.mount_document_root(mounts, mode)
    }

    fn mount_fragment(
        &self,
        html: &'static str,
        mounts: &'static [MountId],
        mode: MountMode<'_>,
    ) -> Result<Mounted, JsValue> {
        let document = document()?;
        if let Some(target) = super::hydration::take_range() {
            target.validate()?;
            let start = target
                .start
                .next_sibling()
                .ok_or_else(|| invalid("missing children start"))?;
            let end = target
                .end
                .previous_sibling()
                .ok_or_else(|| invalid("missing children end"))?;
            if start.node_value().as_deref() != Some("fusor:fragment")
                || end.node_value().as_deref() != Some("/fusor:fragment")
            {
                return Err(invalid("server children fragment mismatch"));
            }
            let fragment = MountPoint { start, end };
            fragment.validate()?;
            let mut scope = mode.scope(document.create_element("div")?);
            scope.fragment = Some(fragment);
            scope.hydrating = true;
            return self.resolve(scope, mounts, mode.bundled());
        }
        let template = parse_html(html)?
            .ok_or_else(|| invalid("missing children template"))?
            .dyn_into::<HtmlTemplateElement>()
            .map_err(|_| invalid("expected a children template"))?;
        let root = document.create_element("div")?;
        root.append_child(&template.content().clone_node_with_deep(true)?)?;
        let (mut scope, nodes) = self.resolve(mode.scope(root), mounts, mode.bundled())?;
        let start: Node = document.create_comment("fusor:fragment").into();
        let end: Node = document.create_comment("/fusor:fragment").into();
        scope
            .root()
            .insert_before(&start, scope.root().first_child().as_ref())?;
        scope.root().append_child(&end)?;
        scope.fragment = Some(MountPoint { start, end });
        Ok((scope, nodes))
    }

    /// No managed child regions or form controls: a native bundle binds it.
    fn is_flat(&self) -> bool {
        let form = |tag| matches!(tag, "input" | "textarea" | "select");
        self.elements
            .iter()
            .all(|element| element.children == ChildPolicy::Static && !form(element.tag))
            && self.text_elements.iter().all(|element| !form(element.tag))
    }

    /// Mount the single document root marked with this component's identity.
    fn mount_document_root(
        &self,
        mounts: &'static [MountId],
        mode: MountMode<'_>,
    ) -> Result<Mounted, JsValue> {
        if mode.bundled() && mounts.is_empty() && self.kind == RootKind::Template && self.is_flat()
        {
            // The same lookup, checks and resolution as below, in one native call.
            let (root, binding_bundle) = flat::mount_document_template(self)?;
            return Ok((mode.scope(root), TemplateNodes::bundle(binding_bundle)));
        }
        let document = document()?;
        let metadata = strings::descriptor(self.component, self.version);
        let roots = metadata.roots(&document)?;
        if roots.length() != 1 {
            return Err(invalid(format_args!(
                "component {} requires exactly one root, found {}",
                self.component,
                roots.length()
            )));
        }
        let root: Element = roots.item(0).expect("one root").dyn_into()?;
        if !metadata.version_matches(&root) {
            return Err(invalid(
                "HTML schema version differs from Wasm; rebuild the application",
            ));
        }
        let scope = match self.kind {
            RootKind::Existing => {
                if root.is_instance_of::<HtmlTemplateElement>() {
                    return Err(invalid("expected an existing element, found a template"));
                }
                mode.scope(root)
            }
            RootKind::Template => {
                let template = root
                    .dyn_into::<HtmlTemplateElement>()
                    .map_err(|_| invalid("expected an HTML template"))?;
                mode.clone_template(&template)?
            }
        };
        self.resolve(scope, mounts, mode.bundled())
    }

    fn resolve(
        &self,
        scope: Scope,
        expected_mounts: &'static [MountId],
        bundled: bool,
    ) -> Result<Mounted, JsValue> {
        if self.version != template::VERSION {
            return Err(invalid(
                "unsupported descriptor version; rebuild the application",
            ));
        }
        let cached = self.kind == RootKind::Template && !scope.hydrating;
        if bundled && expected_mounts.is_empty() && scope.fragment.is_none() && self.is_flat() {
            let binding_bundle = flat::resolve_bundle(self, scope.root(), cached)?;
            strings::descriptor(self.component, self.version).mark_instance(scope.root())?;
            return Ok((scope, TemplateNodes::bundle(binding_bundle)));
        }
        #[cfg(feature = "islands")]
        if scope.hydrating
            && expected_mounts.is_empty()
            && scope.fragment.is_none()
            && self
                .elements
                .iter()
                .all(|element| element.children == ChildPolicy::Static)
        {
            let (handles, slots, mounts) = flat::resolve(self, scope.root())?;
            return self.finish_resolution(scope, handles, slots, mounts);
        }
        if cached {
            if let Some((handles, slots, mounts)) =
                cache::resolve(self, expected_mounts, scope.root())?
            {
                return self.finish_resolution(scope, handles, slots, mounts);
            }
        }
        let (handles, slots, mounts) = scan::resolve(
            self,
            expected_mounts,
            scope.root(),
            scope.fragment.as_ref(),
            scope.is_hydrating(),
        )?;
        if cached {
            // Cache construction is optional; inability to retain an inert
            // certificate must not make a correctly validated mount fail.
            let _ = cache::remember(
                self,
                expected_mounts,
                scope.root(),
                &handles,
                &slots,
                &mounts,
            );
        }
        self.finish_resolution(scope, handles, slots, mounts)
    }

    fn finish_resolution(
        &self,
        scope: Scope,
        handles: Handles,
        slots: Vec<Slot>,
        mounts: Mounts,
    ) -> Result<Mounted, JsValue> {
        let document = document()?;
        let mut texts = Nodes::new();
        for Slot {
            id,
            position,
            existing,
        } in slots
        {
            let text = match existing {
                Some(text) => text,
                None => {
                    let text = document.create_text_node("");
                    match position {
                        TextPosition::Anchored { start, end } => {
                            start
                                .parent_node()
                                .ok_or_else(|| invalid("detached text anchor"))?
                                .insert_before(&text, Some(&end))?;
                        }
                        TextPosition::Element(element) => {
                            element.append_child(&text)?;
                        }
                    }
                    text
                }
            };
            texts.insert(id, text);
        }
        texts.finish();
        strings::descriptor(self.component, self.version).mark_instance(scope.root())?;
        Ok((
            scope,
            TemplateNodes {
                binding_bundle: None,
                elements: handles,
                texts,
                mounts,
            },
        ))
    }
}

fn text_slot(id: TextId, start: &Node, end: &Node) -> Result<Option<Text>, JsValue> {
    let next = start
        .next_sibling()
        .ok_or_else(|| invalid(format_args!("unpaired text slot {id}")))?;
    if next.is_same_node(Some(end)) {
        return Ok(None);
    }
    if !next
        .next_sibling()
        .is_some_and(|node| node.is_same_node(Some(end)))
    {
        return Err(invalid(format_args!("unexpected nodes in text slot {id}")));
    }
    next.dyn_into::<Text>()
        .map(Some)
        .map_err(|_| invalid(format_args!("expected a text node in slot {id}")))
}

/// The compiler proves this host has no static or managed child content.
/// Validate before insertion so a malformed later slot cannot partially bind it.
fn element_text(id: TextId, element: &Element) -> Result<Option<Text>, JsValue> {
    let Some(child) = element.first_child() else {
        return Ok(None);
    };
    if child.next_sibling().is_some() {
        return Err(invalid(format_args!(
            "unexpected nodes in text element {id}"
        )));
    }
    child
        .dyn_into::<Text>()
        .map(Some)
        .map_err(|_| invalid(format_args!("expected a text node in text element {id}")))
}
