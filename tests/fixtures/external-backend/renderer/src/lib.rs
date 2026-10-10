//! A deliberately small renderer contract test, not an application framework.
mod coherent;
pub use coherent::Frame;

use fusor::bind::{Checkbox, TextValue};
use fusor::{Effect, Owner, OwnerHandle, Registration, Signal, batch, effect, signal};
use std::{
    any::Any,
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    fmt::Display,
    rc::Rc,
};

pub type Error = fusor::coherence::Error;
pub const VERSION: u32 = 2;
pub fn error(value: impl Display + 'static) -> Error {
    Error::renderer(value)
}

pub trait Component: Sized + 'static {
    fn prepare(
        parent: Option<&OwnerHandle>,
        make: Box<dyn FnOnce(OwnerHandle) -> Result<Self, Error> + '_>,
        children: Children,
    ) -> Result<Scope, Error>;
}

pub type Children = fusor::render::Children<Scope, Error>;

pub struct StaticNode {
    pub parent: Option<usize>,
    pub kind: Kind,
}
pub enum Kind {
    Element(
        &'static str,
        &'static [(&'static str, &'static str)],
        Option<usize>,
    ),
    Text(&'static str, Option<usize>),
    Mount(usize),
    Comment(&'static str),
}

type Callback = Rc<RefCell<dyn FnMut()>>;
#[derive(Clone)]
pub struct Node(Rc<NodeData>);
struct NodeData {
    id: usize,
    tag: &'static str,
    attributes: BTreeMap<&'static str, &'static str>,
    text: RefCell<String>,
    children: RefCell<Vec<Node>>,
    listeners: RefCell<BTreeMap<String, Vec<Callback>>>,
    value: RefCell<String>,
    checked: Cell<bool>,
}
thread_local! { static NEXT_ID: Cell<usize> = const { Cell::new(1) }; }
impl Node {
    fn new(tag: &'static str, text: &str, attributes: &[(&'static str, &'static str)]) -> Self {
        Self(Rc::new(NodeData {
            id: NEXT_ID.with(|next| {
                let id = next.get();
                next.set(id + 1);
                id
            }),
            tag,
            attributes: attributes.iter().copied().collect(),
            text: RefCell::new(text.into()),
            children: RefCell::new(Vec::new()),
            listeners: RefCell::new(BTreeMap::new()),
            value: RefCell::new(
                attributes
                    .iter()
                    .find(|(name, _)| *name == "value")
                    .map_or("", |(_, value)| *value)
                    .into(),
            ),
            checked: Cell::new(false),
        }))
    }
    pub fn id(&self) -> usize {
        self.0.id
    }
    pub fn tag(&self) -> &str {
        self.0.tag
    }
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.0.attributes.get(name).copied()
    }
    pub fn text(&self) -> String {
        let mut text = self.0.text.borrow().clone();
        for child in self.0.children.borrow().iter() {
            text.push_str(&child.text());
        }
        text
    }
    pub fn find(&self, id: &str) -> Option<Node> {
        if self.attribute("id") == Some(id) {
            return Some(self.clone());
        }
        self.0
            .children
            .borrow()
            .iter()
            .find_map(|child| child.find(id))
    }
    pub fn elements(&self, tag: &str) -> Vec<Node> {
        let mut nodes = if self.tag() == tag {
            vec![self.clone()]
        } else {
            Vec::new()
        };
        for child in self.0.children.borrow().iter() {
            nodes.extend(child.elements(tag));
        }
        nodes
    }
    pub fn dispatch(&self, event: &str) {
        let listeners = self
            .0
            .listeners
            .borrow()
            .get(event)
            .cloned()
            .unwrap_or_default();
        // Each listener batches independently, and no tree borrow spans user code.
        for callback in listeners {
            batch(|| (callback.borrow_mut())());
        }
    }
    pub fn edit(&self, text: &str) {
        *self.0.value.borrow_mut() = text.into();
        self.dispatch("input");
    }
    pub fn value(&self) -> String {
        self.0.value.borrow().clone()
    }
    pub fn blur(&self) {
        self.dispatch("blur");
    }
    pub fn check(&self, value: bool) {
        self.0.checked.set(value);
        self.dispatch("change");
    }
    pub fn checked(&self) -> bool {
        self.0.checked.get()
    }
}

#[derive(Default)]
struct Retained {
    effects: Vec<Effect>,
    states: Vec<Rc<dyn Any>>,
    children: Vec<Scope>,
}
pub struct Scope {
    owner: Owner,
    roots: Vec<Node>,
    elements: BTreeMap<usize, Node>,
    texts: BTreeMap<usize, Node>,
    mounts: BTreeMap<usize, Node>,
    attached: Option<Node>,
    coherent: Option<Rc<coherent::Tree>>,
    retained: Rc<RefCell<Retained>>,
    _cleanup: Registration,
}
impl Drop for Scope {
    fn drop(&mut self) {
        self.owner.dispose();
        if let Some(target) = &self.attached {
            target
                .0
                .children
                .borrow_mut()
                .retain(|node| !self.roots.iter().any(|root| Rc::ptr_eq(&node.0, &root.0)));
        }
    }
}

impl fusor_router::view::RouteScope for Scope {
    type Target = Node;
    type Error = Error;

    fn prepare_at(&mut self, target: &Node, _parent_active: bool) -> Result<(), Error> {
        if self.attached.is_some() {
            return Err("route view is already attached".into());
        }
        target.0.children.borrow_mut().extend(self.roots.clone());
        self.attached = Some(target.clone());
        Ok(())
    }

    fn commit(&self) {
        self.publish();
    }

    fn error(message: &str) -> Error {
        message.into()
    }
}
impl fusor::render::Scope for Scope {
    fn owner(&self) -> OwnerHandle {
        self.owner()
    }

    fn retain_state<T: 'static>(&mut self, state: T) -> Rc<T> {
        self.retain_state(state)
    }

    fn prepares_effects(&self) -> bool {
        self.is_coherent()
    }
}
impl Scope {
    pub fn new(parent: Option<&OwnerHandle>, specs: &[StaticNode]) -> Result<Self, Error> {
        let owner = parent.map_or_else(Owner::new, Owner::child);
        let retained = Rc::new(RefCell::new(Retained::default()));
        let weak = Rc::downgrade(&retained);
        let mut all: Vec<Node> = Vec::new();
        let (mut roots, mut elements, mut texts, mut mounts) = (
            Vec::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        );
        for spec in specs {
            let (node, anchor) = match &spec.kind {
                Kind::Element(tag, attrs, anchor) => (
                    Node::new(tag, "", attrs),
                    anchor.map(|id| (&mut elements, id)),
                ),
                Kind::Text(text, anchor) => (
                    Node::new("#text", text, &[]),
                    anchor.map(|id| (&mut texts, id)),
                ),
                Kind::Mount(id) => (Node::new("#mount", "", &[]), Some((&mut mounts, *id))),
                Kind::Comment(_) => (Node::new("#comment", "", &[]), None),
            };
            if let Some((target, id)) = anchor {
                target.insert(id, node.clone());
            }
            if let Some(parent) = spec.parent {
                all.get(parent)
                    .ok_or("static tree parent must precede child")?
                    .0
                    .children
                    .borrow_mut()
                    .push(node.clone());
            } else {
                roots.push(node.clone());
            }
            all.push(node);
        }
        let coherent = coherent::Tree::inherited(&owner.handle(), &elements, &texts, &mounts);
        let weak_tree = coherent.as_ref().map(Rc::downgrade);
        let cleanup = owner.handle().on_cleanup(move || {
            for node in &all {
                node.0.listeners.borrow_mut().clear();
            }
            if let Some(retained) = weak.upgrade() {
                let discarded = std::mem::take(&mut *retained.borrow_mut());
                drop(discarded);
            }
            if let Some(tree) = weak_tree.and_then(|tree| tree.upgrade()) {
                tree.clear();
            }
        });
        Ok(Self {
            owner,
            roots,
            elements,
            texts,
            mounts,
            attached: None,
            coherent,
            retained,
            _cleanup: cleanup,
        })
    }
    pub fn owner(&self) -> OwnerHandle {
        self.owner.handle()
    }
    pub fn retain_state<T: 'static>(&mut self, state: T) -> Rc<T> {
        let state = Rc::new(state);
        self.retained.borrow_mut().states.push(state.clone());
        state
    }
    pub fn publish(&self) {
        self.owner.commit();
    }
    pub fn dispose(&self) {
        self.owner.dispose();
    }
    pub fn root(&self) -> Node {
        self.roots
            .iter()
            .find(|node| !node.tag().starts_with('#'))
            .expect("element root")
            .clone()
    }
    fn keep(&mut self, effect: Effect) {
        self.retained.borrow_mut().effects.push(effect);
    }
    fn try_effect(
        &mut self,
        mut update: impl FnMut() -> Result<(), Error> + 'static,
    ) -> Result<(), Error> {
        let failure = Rc::new(RefCell::new(None));
        let mut initial = Some(failure.clone());
        let subscription = effect(move || {
            let result = update();
            if let Some(initial) = initial.take() {
                *initial.borrow_mut() = result.err();
            } else {
                result.expect("reactive component update");
            }
        });
        if let Some(error) = failure.take() {
            return Err(error);
        }
        self.keep(subscription);
        Ok(())
    }
    fn element(&self, id: usize) -> Result<Node, Error> {
        self.elements
            .get(&id)
            .cloned()
            .ok_or_else(|| Error::from(format!("missing element anchor {id}")))
    }
    fn mount(&self, id: usize) -> Result<Node, Error> {
        self.mounts
            .get(&id)
            .cloned()
            .ok_or_else(|| Error::from(format!("missing mount anchor {id}")))
    }
    pub fn text<T: Display>(
        &mut self,
        id: usize,
        read: impl Fn() -> T + 'static,
    ) -> Result<(), Error> {
        let node = self.texts.get(&id).cloned().ok_or("missing text anchor")?;
        self.keep(effect(move || {
            *node.0.text.borrow_mut() = read().to_string()
        }));
        Ok(())
    }
    pub fn on(
        &mut self,
        id: usize,
        event: &str,
        callback: impl FnMut(()) + 'static,
    ) -> Result<(), Error> {
        let node = self.element(id)?;
        let mut guarded = self.owner().guarded(callback);
        node.0
            .listeners
            .borrow_mut()
            .entry(event.into())
            .or_default()
            .push(Rc::new(RefCell::new(move || {
                guarded(());
            })));
        Ok(())
    }
    pub fn bind_text(&mut self, id: usize, value: impl TextValue) -> Result<(), Error> {
        let node = self.element(id)?;
        let (input, write) = (node.clone(), value.clone());
        self.on(id, "input", move |()| write.edit(input.value()))?;
        let touch = value.clone();
        self.on(id, "blur", move |()| touch.touch())?;
        self.keep(effect(move || {
            if !value.shows(&node.value()) {
                *node.0.value.borrow_mut() = value.text();
            }
        }));
        Ok(())
    }
    pub fn bind_checkbox(
        &mut self,
        id: usize,
        value: impl Checkbox,
        choice: impl Fn() -> String + 'static,
    ) -> Result<(), Error> {
        let node = self.element(id)?;
        let choice = Rc::new(choice);
        let (input, write, input_choice) = (node.clone(), value.clone(), choice.clone());
        self.on(id, "change", move |()| {
            write.check(&input_choice(), input.checked())
        })?;
        self.keep(effect(move || node.0.checked.set(value.checked(&choice()))));
        Ok(())
    }
    pub fn branch<T: Clone + PartialEq + 'static>(
        &mut self,
        id: usize,
        read: impl Fn() -> (usize, T) + 'static,
        prepare: impl Fn(usize, Signal<T>, &OwnerHandle) -> Result<Scope, Error> + 'static,
    ) -> Result<(), Error> {
        let (node, owner) = (self.mount(id)?, self.owner());
        let mut current: Option<(usize, Signal<T>, Scope)> = None;
        self.keep(effect(move || {
            let (index, value) = read();
            if let Some((old, data, _)) = &current {
                if *old == index {
                    data.set(value);
                    return;
                }
            }
            let data = signal(value);
            let scope = fusor::untrack(|| prepare(index, data.clone(), &owner))
                .expect("branch preparation");
            *node.0.children.borrow_mut() = scope.roots.clone();
            current = Some((index, data, scope));
            current.as_ref().unwrap().2.publish();
        }));
        Ok(())
    }
    pub fn keyed<T: Clone + PartialEq + 'static, K: Ord + Clone + 'static>(
        &mut self,
        id: usize,
        read: impl Fn() -> Vec<T> + 'static,
        key: impl Fn(&T) -> K + 'static,
        prepare: impl Fn(Signal<T>, &OwnerHandle) -> Result<Scope, Error> + 'static,
    ) -> Result<(), Error> {
        let (node, owner) = (self.element(id)?, self.owner());
        let mut rows: BTreeMap<K, (Signal<T>, Scope)> = BTreeMap::new();
        self.keep(effect(move || {
            let entries: Vec<_> = read()
                .into_iter()
                .map(|value| (key(&value), value))
                .collect();
            let mut keys = BTreeSet::new();
            for (key, _) in &entries {
                assert!(keys.insert(key), "duplicate row key");
            }
            let mut prepared = BTreeMap::new();
            for (key, value) in &entries {
                if !rows.contains_key(key) {
                    let data = signal(value.clone());
                    let scope =
                        fusor::untrack(|| prepare(data.clone(), &owner)).expect("row preparation");
                    prepared.insert(key.clone(), (data, scope));
                }
            }
            // No retained row changes or activation precede complete preparation.
            // Batch input updates so observers cannot reenter a half-published list.
            batch(|| {
                let mut next = BTreeMap::new();
                let mut children = Vec::new();
                let order: Vec<_> = entries.iter().map(|(key, _)| key.clone()).collect();
                for (key, value) in entries {
                    let row = rows.remove(&key).or_else(|| prepared.remove(&key)).unwrap();
                    row.0.set(value);
                    children.extend(row.1.roots.clone());
                    next.insert(key, row);
                }
                *node.0.children.borrow_mut() = children;
                rows = next;
                for key in order {
                    rows[&key].1.publish();
                }
            });
        }));
        Ok(())
    }
    pub fn component<
        T: Component,
        K: Clone + PartialEq + 'static,
        I: Fn() -> Option<K> + 'static,
        M: Fn(OwnerHandle) -> Result<T, Error> + 'static,
    >(
        &mut self,
        id: usize,
        identity: I,
        make: M,
        children: Children,
    ) -> Result<(), Error> {
        let (node, owner) = (self.mount(id)?, self.owner());
        let mut current: Option<(K, Scope)> = None;
        self.try_effect(move || {
            let identity = identity();
            if current.as_ref().map(|(key, _)| key) == identity.as_ref() {
                return Ok(());
            }
            let next = identity
                .map(|key| {
                    fusor::untrack(|| T::prepare(Some(&owner), Box::new(&make), children.clone()))
                        .map(|scope| (key, scope))
                })
                .transpose()?;
            *node.0.children.borrow_mut() = next
                .as_ref()
                .map_or_else(Vec::new, |(_, scope)| scope.roots.clone());
            current = next;
            if let Some((_, scope)) = &current {
                scope.publish();
            }
            Ok(())
        })
    }
    pub fn children(&mut self, id: usize, children: Children) -> Result<(), Error> {
        if let Some(scope) = fusor::untrack(|| children.prepare(&self.owner()))? {
            *self.mount(id)?.0.children.borrow_mut() = scope.roots.clone();
            scope.publish();
            self.retained.borrow_mut().children.push(scope);
        }
        Ok(())
    }

    pub fn routes(
        &mut self,
        id: usize,
        routes: Vec<fusor_router::view::RouteView<Self>>,
    ) -> Result<(), Error> {
        let router = fusor_router::view::ViewRouter::mount(
            &self.owner(),
            &self.mount(id)?,
            routes,
            fusor_router::AppUrl::parse("/").map_err(error)?,
        )?;
        self.retain_state(router);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(
        parent: Option<&OwnerHandle>,
        tag: &'static str,
        attributes: &'static [(&'static str, &'static str)],
    ) -> Scope {
        Scope::new(
            parent,
            &[StaticNode {
                parent: None,
                kind: Kind::Element(tag, attributes, Some(0)),
            }],
        )
        .unwrap()
    }

    #[test]
    fn constructor_reads_do_not_subscribe_the_list_effect() {
        let mut scope = element(None, "ul", &[]);
        let values = signal(vec![1]);
        let unrelated = signal(0);
        let reads = Rc::new(Cell::new(0));
        let constructions = Rc::new(Cell::new(0));
        let (items, read_count) = (values.clone(), reads.clone());
        let (constructor_input, construction_count) = (unrelated.clone(), constructions.clone());
        scope
            .keyed(
                0,
                move || {
                    read_count.set(read_count.get() + 1);
                    items.get()
                },
                |value| *value,
                move |_, owner| {
                    let _input = constructor_input.get();
                    construction_count.set(construction_count.get() + 1);
                    Scope::new(Some(owner), &[])
                },
            )
            .unwrap();
        scope.publish();
        assert_eq!((reads.get(), constructions.get()), (1, 1));
        unrelated.set(1);
        assert_eq!((reads.get(), constructions.get()), (1, 1));
        values.set(vec![1, 2]);
        assert_eq!((reads.get(), constructions.get()), (2, 2));
        unrelated.set(2);
        assert_eq!((reads.get(), constructions.get()), (2, 2));
    }

    #[test]
    fn text_field_preserves_drafts_and_receives_touch() {
        let mut scope = element(None, "input", &[("value", "static")]);
        assert_eq!(scope.root().value(), "static");
        let field = fusor_std::forms::TextField::<u32>::new(42);
        scope.bind_text(0, field.clone()).unwrap();
        scope.publish();
        let node = scope.root();
        node.edit("-");
        assert_eq!(node.value(), "-");
        assert_eq!(field.raw(), "-");
        assert!(field.parsed().is_err());
        assert!(!field.touched());
        node.blur();
        assert!(field.touched());
        field.reset(8);
        assert_eq!(node.value(), "8");
    }

    #[test]
    fn list_activation_observes_published_scene() {
        let mut scope = element(None, "ul", &[]);
        let node = scope.root();
        let values = signal(Vec::<i32>::new());
        let read = values.clone();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let observed = seen.clone();
        scope
            .keyed(
                0,
                move || read.get(),
                |value| *value,
                move |value, owner| {
                    let mut row = element(Some(owner), "li", &[]);
                    let (node, observed, id) =
                        (node.clone(), observed.clone(), value.get_untracked());
                    let activation = row.owner().on_activate(move || {
                        observed.borrow_mut().push((id, node.elements("li").len()))
                    });
                    row.retain_state(activation);
                    Ok(row)
                },
            )
            .unwrap();
        scope.publish();
        values.set(vec![2, 1]);
        assert_eq!(*seen.borrow(), vec![(2, 2), (1, 2)]);
    }

    #[test]
    fn rejected_list_updates_do_not_activate_or_publish_prepared_rows() {
        for duplicate in [false, true] {
            let mut scope = element(None, "ul", &[]);
            let values = signal(Vec::<i32>::new());
            let read = values.clone();
            let activated = Rc::new(Cell::new(0));
            let observed = activated.clone();
            scope
                .keyed(
                    0,
                    move || read.get(),
                    |value| *value,
                    move |value, owner| {
                        if value.get() == 2 {
                            return Err("expected row preparation failure".into());
                        }
                        let mut row = element(Some(owner), "li", &[]);
                        let observed = observed.clone();
                        let activation = row
                            .owner()
                            .on_activate(move || observed.set(observed.get() + 1));
                        row.retain_state(activation);
                        Ok(row)
                    },
                )
                .unwrap();
            scope.publish();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                values.set(if duplicate { vec![1, 1] } else { vec![1, 2] });
            }));
            assert!(result.is_err());
            assert_eq!(activated.get(), 0);
            assert!(scope.root().elements("li").is_empty());
        }
    }

    #[test]
    fn each_listener_batches_its_own_updates() {
        let mut scope = element(None, "button", &[]);
        let value = signal(0);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let (read, observed) = (value.clone(), seen.clone());
        let _effect = effect(move || observed.borrow_mut().push(read.get()));
        let first = value.clone();
        scope
            .on(0, "click", move |()| {
                first.set(1);
                first.set(2);
            })
            .unwrap();
        scope
            .on(0, "click", move |()| {
                value.set(3);
                value.set(4);
            })
            .unwrap();
        scope.publish();
        scope.root().dispatch("click");
        assert_eq!(*seen.borrow(), vec![0, 2, 4]);
    }
}
