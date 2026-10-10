//! Owned patch preparation for generated coherent HTML. No application closure
//! runs between the first and last DOM patch.
mod interaction;
mod structure;

use interaction::BlockingOverlay;

use super::{JsValue, Listener, MountPoint, Scope, is_html};
use crate::{
    ContextKey, OwnerHandle,
    coherence::{AsyncBoundary, Attempt, Error, Publication},
};
use std::{any::Any, cell::RefCell, collections::BTreeMap, rc::Rc};
use web_sys::{Element, Event, Node, Text};

type Renderer = dyn Fn(&mut Frame<'_>) -> Result<(), Error>;

/// A coherent binding's stable slot and validated DOM target.
#[doc(hidden)]
pub struct RenderSlot<'a, T> {
    pub index: usize,
    pub target: &'a T,
}
struct Context;
impl ContextKey for Context {
    type Value = BoundaryContext;
}

#[derive(Clone)]
struct BoundaryContext {
    boundary: AsyncBoundary,
    overlay: BlockingOverlay,
}

// Element and mount-point identifiers have separate compiler namespaces.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SlotId {
    List(usize),
    Component(usize),
    Children(usize),
    Branch(usize),
}

pub(super) struct Tree {
    root: Element,
    fragment: Option<MountPoint>,
    context: BoundaryContext,
    owner: OwnerHandle,
    renderer: RefCell<Option<Rc<Renderer>>>,
    slots: RefCell<BTreeMap<SlotId, Rc<dyn Any>>>,
    listeners: RefCell<Vec<Listener>>,
}

impl Tree {
    fn new(scope: &Scope, context: BoundaryContext, renderer: Option<Rc<Renderer>>) -> Rc<Self> {
        Rc::new(Self {
            root: scope.root.clone(),
            fragment: scope.fragment.clone(),
            context,
            owner: scope.owner(),
            renderer: RefCell::new(renderer),
            slots: RefCell::new(BTreeMap::new()),
            listeners: RefCell::new(Vec::new()),
        })
    }
}

// Query only inherited metadata. Integration setup still follows successful
// native validation in the descriptor preparation entry point.
pub(super) fn parent_is_coherent(parent: Option<&OwnerHandle>) -> bool {
    parent
        .and_then(|owner| owner.context::<Context>())
        .is_some()
}

impl Scope {
    pub(super) fn prepare_coherent(&mut self, parent: Option<&OwnerHandle>) {
        self.render_tree = parent
            .and_then(|owner| owner.context::<Context>())
            .map(|context| Tree::new(self, (*context).clone(), None));
    }

    #[doc(hidden)]
    pub fn is_coherent(&self) -> bool {
        self.render_tree.is_some()
    }

    #[doc(hidden)]
    pub fn set_coherent_renderer(
        &mut self,
        render: impl Fn(&mut Frame<'_>) -> Result<(), Error> + 'static,
    ) {
        *self
            .render_tree
            .as_ref()
            .expect("coherent scope")
            .renderer
            .borrow_mut() = Some(Rc::new(render));
    }

    #[doc(hidden)]
    pub fn async_region(
        &mut self,
        root: &Element,
        boundary: AsyncBoundary,
        render: impl Fn(&mut Frame<'_>) -> Result<(), Error> + 'static,
    ) -> Result<(), JsValue> {
        if self.is_coherent() {
            return Err(JsValue::from_str(
                "nested coherent boundaries are unsupported",
            ));
        }
        #[cfg(feature = "islands")]
        super::delivery::register_preview_boundary(&self.owner(), &boundary);
        let mut region = Scope::new(root.clone());
        region.prepare_owner(Some(&self.owner()));
        let overlay = BlockingOverlay::new(root);
        let context = BoundaryContext {
            boundary: boundary.clone(),
            overlay: overlay.clone(),
        };
        region
            .owner()
            .provide::<Context>(context.clone())
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let tree = Tree::new(&region, context, Some(Rc::new(render)));
        region.render_tree = Some(tree.clone());
        let captured_root = root.clone();
        let mounted = boundary
            .attach(&region.owner(), move |attempt| {
                let mut publication = Prepared::default();
                visit(&tree, attempt, &mut publication)?;
                publication.roots.push(captured_root.clone());
                Ok(Box::new(publication))
            })
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        region.retain(mounted);
        overlay.install(&mut region, boundary)?;
        region.retain(overlay);
        // Region has no ordinary DOM side effects. New descendants remain
        // prepared until their publication's finish phase.
        region.try_commit()?;
        self.children.push(region);
        Ok(())
    }
}

fn error(value: JsValue) -> Error {
    Error::renderer(DomError(value))
}

/// A native DOM failure retained by a coherent renderer error. Downcast the
/// coherent error to this type to inspect the browser exception.
#[derive(Debug)]
pub struct DomError(pub JsValue);
impl std::fmt::Display for DomError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.as_string() {
            Some(message) => formatter.write_str(&message),
            None => write!(formatter, "{:?}", self.0),
        }
    }
}
impl std::error::Error for DomError {}
fn allowed(element: &Element) -> Result<(), Error> {
    if element.local_name().contains('-') || element.has_attribute("is") || !is_html(element) {
        return Err(
            "custom elements and foreign DOM cannot participate in coherent patches".into(),
        );
    }
    Ok(())
}

enum Patch {
    Inert(BoundaryContext, bool, bool),
    Text(Text, String, String),
    Attribute(Element, String, Option<String>, Option<String>),
    Class(Element, String, bool, bool),
}
impl Patch {
    fn apply(&self, reverse: bool) -> Result<(), Error> {
        fn pick<T>(reverse: bool, next: T, old: T) -> T {
            if reverse { old } else { next }
        }
        match self {
            Self::Inert(context, next, old) => context
                .overlay
                .set_authored_inert(pick(reverse, *next, *old))
                .map_err(error),
            Self::Text(node, next, old) => {
                node.set_data(pick(reverse, next, old));
                Ok(())
            }
            Self::Attribute(node, name, next, old) => match pick(reverse, next, old) {
                Some(value) => node.set_attribute(name, value).map_err(error),
                None => node.remove_attribute(name).map_err(error),
            },
            Self::Class(node, name, next, old) => node
                .class_list()
                .toggle_with_force(name, pick(reverse, *next, *old))
                .map(|_| ())
                .map_err(error),
        }
    }
}

trait Structure {
    fn validate(&self) -> Result<(), Error>;
    fn apply(&self) -> Result<(), Error>;
    fn finish(self: Box<Self>);
}
#[derive(Default)]
struct Prepared {
    patches: Vec<Patch>,
    structures: Vec<Box<dyn Structure>>,
    listeners: Vec<(Rc<Tree>, Vec<Listener>)>,
    roots: Vec<Element>,
    targets: Vec<(Rc<Tree>, Node)>,
}
impl Publication for Prepared {
    fn validate(&self) -> Result<(), Error> {
        for root in &self.roots {
            allowed(root)?;
        }
        for (root, target) in &self.targets {
            if !root.fragment.as_ref().map_or_else(
                || root.root.contains(Some(target)),
                |point| point.contains(target),
            ) {
                return Err("coherent target was moved outside its component".into());
            }
        }
        for structure in &self.structures {
            structure.validate()?;
        }
        Ok(())
    }
    fn apply(&mut self) -> Result<(), Error> {
        for (index, patch) in self.patches.iter().enumerate() {
            if let Err(error) = patch.apply(false) {
                for patch in self.patches[..index].iter().rev() {
                    if let Err(rollback) = patch.apply(true) {
                        web_sys::console::error_1(&JsValue::from_str(&rollback.to_string()));
                    }
                }
                return Err(error);
            }
        }
        for structure in &self.structures {
            structure.apply()?;
        }
        Ok(())
    }
    fn finish(self: Box<Self>) {
        for (tree, listeners) in self.listeners {
            tree.listeners.replace(listeners);
        }
        for structure in self.structures {
            structure.finish();
        }
    }
}

#[doc(hidden)]
pub struct Frame<'a> {
    pub attempt: &'a Attempt,
    tree: &'a Rc<Tree>,
    publication: &'a mut Prepared,
    listeners: Vec<Listener>,
}
fn visit(tree: &Rc<Tree>, attempt: &Attempt, publication: &mut Prepared) -> Result<(), Error> {
    if tree.owner.is_disposed() {
        return Err("coherent component disposed during preparation".into());
    }
    let renderer = tree
        .renderer
        .borrow()
        .clone()
        .ok_or("component has no generated coherent renderer")?;
    let mut frame = Frame {
        attempt,
        tree,
        publication,
        listeners: Vec::new(),
    };
    renderer(&mut frame)?;
    let listeners = std::mem::take(&mut frame.listeners);
    frame.publication.listeners.push((tree.clone(), listeners));
    Ok(())
}

impl Frame<'_> {
    pub fn reject(&self, reason: &str) -> Result<(), Error> {
        Err(reason.into())
    }
    /// Require `node` to stay inside this component until publication.
    fn target(&mut self, node: &Node) {
        self.publication
            .targets
            .push((self.tree.clone(), node.clone()));
    }
    pub fn text(&mut self, node: &Text, value: impl ToString) -> Result<(), Error> {
        self.target(node);
        let next = value.to_string();
        let old = node.data();
        if next != old {
            self.publication
                .patches
                .push(Patch::Text(node.clone(), next, old));
        }
        Ok(())
    }
    pub fn attr(&mut self, node: &Element, name: &str, next: Option<String>) -> Result<(), Error> {
        allowed(node)?;
        self.target(node);
        if name == "inert" && self.tree.context.overlay.owns_root(node) {
            let old = self.tree.context.overlay.authored_inert();
            self.publication.patches.push(Patch::Inert(
                self.tree.context.clone(),
                next.is_some(),
                old,
            ));
            return Ok(());
        }
        let old = node.get_attribute(name);
        if old != next {
            self.publication.patches.push(Patch::Attribute(
                node.clone(),
                name.to_owned(),
                next,
                old,
            ));
        }
        Ok(())
    }
    pub fn class(&mut self, node: &Element, name: &str, next: bool) -> Result<(), Error> {
        allowed(node)?;
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            return Err("invalid coherent class name".into());
        }
        self.target(node);
        let old = node.class_list().contains(name);
        if old != next {
            self.publication
                .patches
                .push(Patch::Class(node.clone(), name.to_owned(), next, old));
        }
        Ok(())
    }
    pub fn on(
        &mut self,
        node: &Element,
        event: &str,
        handler: impl FnMut(Event) + 'static,
    ) -> Result<(), Error> {
        allowed(node)?;
        let owner = self.tree.owner.clone();
        let boundary = self.tree.context.boundary.clone();
        let active = move || owner.is_active() && boundary.is_interactive();
        let listener = Listener::batched(
            super::ListenerTarget::Node(node.clone().into()),
            event,
            active,
            handler,
        )
        .map_err(error)?;
        self.listeners.push(listener);
        Ok(())
    }
}
