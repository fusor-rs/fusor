use super::*;
use fusor::{
    ContextKey,
    coherence::{AsyncBoundary, Attempt, Publication},
    versions::Versions,
};

type Renderer = dyn Fn(&mut Frame<'_>) -> Result<(), Error>;
type Listeners = BTreeMap<String, Vec<Callback>>;
struct Context;
impl ContextKey for Context {
    type Value = AsyncBoundary;
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SlotId {
    Branch(usize),
    Rows(usize),
    Component(usize),
    Children(usize),
}

pub(super) struct Tree {
    owner: OwnerHandle,
    boundary: AsyncBoundary,
    elements: BTreeMap<usize, Node>,
    texts: BTreeMap<usize, Node>,
    mounts: BTreeMap<usize, Node>,
    renderer: RefCell<Option<Rc<Renderer>>>,
    slots: RefCell<BTreeMap<SlotId, Rc<dyn Any>>>,
}
impl Tree {
    pub(super) fn inherited(
        owner: &OwnerHandle,
        elements: &BTreeMap<usize, Node>,
        texts: &BTreeMap<usize, Node>,
        mounts: &BTreeMap<usize, Node>,
    ) -> Option<Rc<Self>> {
        owner.context::<Context>().map(|boundary| {
            Rc::new(Self {
                owner: owner.clone(),
                boundary: (*boundary).clone(),
                elements: elements.clone(),
                texts: texts.clone(),
                mounts: mounts.clone(),
                renderer: RefCell::new(None),
                slots: RefCell::new(BTreeMap::new()),
            })
        })
    }
    pub(super) fn clear(&self) {
        let slots = self.slots.take();
        let renderer = self.renderer.take();
        for node in self.elements.values() {
            let listeners = node.0.listeners.take();
            drop(listeners);
        }
        drop((slots, renderer));
    }
}

impl Scope {
    pub fn is_coherent(&self) -> bool {
        self.coherent.is_some()
    }

    pub fn set_coherent_renderer(
        &mut self,
        render: impl Fn(&mut Frame<'_>) -> Result<(), Error> + 'static,
    ) {
        self.coherent
            .as_ref()
            .expect("coherent scope")
            .renderer
            .replace(Some(Rc::new(render)));
    }

    pub fn async_region(
        &mut self,
        id: usize,
        boundary: AsyncBoundary,
        render: impl Fn(&mut Frame<'_>) -> Result<(), Error> + 'static,
    ) -> Result<(), Error> {
        if self.is_coherent() {
            return Err("nested coherent boundaries are unsupported".into());
        }
        let root = self.element(id)?;
        let mut region = Scope::new(Some(&self.owner()), &[])?;
        region
            .owner()
            .provide::<Context>(boundary.clone())
            .map_err(error)?;
        fn contains(root: &Node, node: &Node) -> bool {
            Rc::ptr_eq(&root.0, &node.0)
                || root
                    .0
                    .children
                    .borrow()
                    .iter()
                    .any(|child| contains(child, node))
        }
        let within = |nodes: &BTreeMap<usize, Node>| {
            nodes
                .iter()
                .filter(|(_, node)| contains(&root, node))
                .map(|(id, node)| (*id, node.clone()))
                .collect()
        };
        let tree = Tree::inherited(
            &region.owner(),
            &within(&self.elements),
            &within(&self.texts),
            &within(&self.mounts),
        )
        .expect("provided boundary");
        region.coherent = Some(tree.clone());
        region.set_coherent_renderer(render);
        let weak = Rc::downgrade(&tree);
        region.retain_state(region.owner().on_cleanup(move || {
            if let Some(tree) = weak.upgrade() {
                tree.clear();
            }
        }));
        let mounted = boundary.attach(&region.owner(), move |attempt| {
            let mut publication = Prepared::default();
            visit(&tree, attempt, &mut publication)?;
            Ok(Box::new(publication))
        })?;
        region.retain_state(mounted);
        region.publish();
        self.retained.borrow_mut().children.push(region);
        Ok(())
    }
}

// Swapping retains the old values until finish, including callback captures whose
// destructors may run application code.
enum Patch {
    Text(Node, String),
    Children(Node, Vec<Node>),
    Listeners(Node, Listeners),
}
#[derive(Default)]
struct Prepared {
    owners: Vec<OwnerHandle>,
    patches: Vec<Patch>,
    finish: Vec<Box<dyn FnOnce()>>,
}
impl Publication for Prepared {
    fn validate(&self) -> Result<(), Error> {
        if self.owners.iter().any(OwnerHandle::is_disposed) {
            Err("coherent scope disposed before publication".into())
        } else {
            Ok(())
        }
    }
    fn apply(&mut self) -> Result<(), Error> {
        for patch in &mut self.patches {
            match patch {
                Patch::Text(node, value) => std::mem::swap(&mut *node.0.text.borrow_mut(), value),
                Patch::Children(node, children) => {
                    std::mem::swap(&mut *node.0.children.borrow_mut(), children)
                }
                Patch::Listeners(node, listeners) => {
                    std::mem::swap(&mut *node.0.listeners.borrow_mut(), listeners)
                }
            }
        }
        Ok(())
    }
    fn finish(self: Box<Self>) {
        for finish in self.finish {
            finish();
        }
    }
}

pub struct Frame<'a> {
    pub attempt: &'a Attempt,
    tree: &'a Rc<Tree>,
    publication: &'a mut Prepared,
    listeners: BTreeMap<usize, Listeners>,
}
fn visit(tree: &Rc<Tree>, attempt: &Attempt, publication: &mut Prepared) -> Result<(), Error> {
    if tree.owner.is_disposed() {
        return Err("coherent scope disposed during evaluation".into());
    }
    let renderer = tree
        .renderer
        .borrow()
        .clone()
        .ok_or("missing generated coherent renderer")?;
    publication.owners.push(tree.owner.clone());
    let mut frame = Frame {
        attempt,
        tree,
        publication,
        listeners: BTreeMap::new(),
    };
    renderer(&mut frame)?;
    for (id, node) in &tree.elements {
        frame.publication.patches.push(Patch::Listeners(
            node.clone(),
            frame.listeners.remove(id).unwrap_or_default(),
        ));
    }
    Ok(())
}

struct Slot<V> {
    epoch: u64,
    current: V,
    candidate: V,
}
impl<V: Default> Default for Slot<V> {
    fn default() -> Self {
        Self {
            epoch: 0,
            current: V::default(),
            candidate: V::default(),
        }
    }
}
type Child<K> = Option<(K, Rc<Scope>)>;
type Row<T> = (Signal<T>, Rc<Scope>);

impl Frame<'_> {
    pub fn reject(&self, reason: &str) -> Result<(), Error> {
        Err(reason.into())
    }

    fn slot<V: Default + 'static>(&self, id: SlotId) -> Result<Rc<RefCell<Slot<V>>>, Error> {
        let erased = self
            .tree
            .slots
            .borrow_mut()
            .entry(id)
            .or_insert_with(|| Rc::new(RefCell::new(Slot::<V>::default())))
            .clone();
        let slot = erased
            .downcast::<RefCell<Slot<V>>>()
            .map_err(|_| "coherent slot type changed")?;
        let discarded = {
            let mut state = slot.borrow_mut();
            if state.epoch != self.attempt.epoch() {
                state.epoch = self.attempt.epoch();
                Some(std::mem::take(&mut state.candidate))
            } else {
                None
            }
        };
        if discarded.is_some() {
            let weak = Rc::downgrade(&slot);
            self.attempt.on_invalidate(move || {
                if let Some(slot) = weak.upgrade() {
                    let candidate = {
                        let mut slot = slot.borrow_mut();
                        slot.epoch = 0;
                        std::mem::take(&mut slot.candidate)
                    };
                    drop(candidate);
                }
            });
        }
        drop(discarded);
        Ok(slot)
    }
    fn publish<V: Default + Clone + 'static>(
        &mut self,
        slot: Rc<RefCell<Slot<V>>>,
        next: V,
        finish: impl FnOnce() + 'static,
    ) {
        let discarded = std::mem::replace(&mut slot.borrow_mut().candidate, next.clone());
        drop(discarded);
        self.publication.finish.push(Box::new(move || {
            let old = {
                let mut slot = slot.borrow_mut();
                let old = std::mem::replace(&mut slot.current, next);
                let candidate = std::mem::take(&mut slot.candidate);
                (old, candidate)
            };
            finish();
            drop(old);
        }));
    }
    fn scope(&mut self, scope: &Scope) -> Result<(), Error> {
        if !scope.owner().is_child_of(&self.tree.owner) {
            return Err("coherent child must belong to its mounting owner".into());
        }
        visit(
            scope
                .coherent
                .as_ref()
                .ok_or("coherent child requires generated bindings")?,
            self.attempt,
            self.publication,
        )
    }
    fn target(&self, id: usize, element: bool) -> Result<Node, Error> {
        (if element {
            &self.tree.elements
        } else {
            &self.tree.mounts
        })
        .get(&id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "missing coherent {} anchor {id}",
                if element { "element" } else { "mount" }
            )
            .into()
        })
    }
    pub fn text<T: Display>(&mut self, id: usize, read: impl Fn() -> T) -> Result<(), Error> {
        let node = self
            .tree
            .texts
            .get(&id)
            .cloned()
            .ok_or("missing coherent text anchor")?;
        self.publication
            .patches
            .push(Patch::Text(node, read().to_string()));
        Ok(())
    }
    pub fn on(
        &mut self,
        id: usize,
        event: &str,
        mut callback: impl FnMut(()) + 'static,
    ) -> Result<(), Error> {
        self.target(id, true)?;
        let (owner, boundary) = (self.tree.owner.clone(), self.tree.boundary.clone());
        self.listeners
            .entry(id)
            .or_default()
            .entry(event.into())
            .or_default()
            .push(Rc::new(RefCell::new(move || {
                if owner.is_active() && boundary.is_interactive() {
                    fusor::untrack(|| callback(()));
                }
            })));
        Ok(())
    }
    pub fn component<T, K, I, M>(
        &mut self,
        id: usize,
        identity: I,
        make: M,
        children: Children,
    ) -> Result<(), Error>
    where
        T: Component,
        K: Clone + PartialEq + 'static,
        I: Fn() -> Option<K>,
        M: Fn(OwnerHandle) -> Result<T, Error>,
    {
        let target = self.target(id, false)?;
        let slot = self.slot::<Child<K>>(SlotId::Component(id))?;
        let next = identity()
            .map(|key| {
                let existing = {
                    let state = slot.borrow();
                    [&state.current, &state.candidate]
                        .into_iter()
                        .find_map(|entry| {
                            entry
                                .as_ref()
                                .filter(|(old, _)| old == &key)
                                .map(|(_, scope)| scope.clone())
                        })
                };
                let scope = match existing {
                    Some(scope) => scope,
                    None => Rc::new(fusor::untrack(|| {
                        T::prepare(Some(&self.tree.owner), Box::new(&make), children)
                    })?),
                };
                self.scope(&scope)?;
                Ok::<_, Error>((key, scope))
            })
            .transpose()?;
        self.publication.patches.push(Patch::Children(
            target,
            next.as_ref()
                .map_or_else(Vec::new, |(_, scope)| scope.roots.clone()),
        ));
        let activated = next.as_ref().map(|(_, scope)| scope.clone());
        self.publish(slot, next, move || {
            if let Some(scope) = activated {
                scope.publish();
            }
        });
        Ok(())
    }
    pub fn branch<T: Clone + PartialEq + 'static>(
        &mut self,
        id: usize,
        read: impl Fn() -> (usize, T),
        prepare: impl Fn(usize, Signal<T>, &OwnerHandle) -> Result<Scope, Error>,
    ) -> Result<(), Error> {
        let target = self.target(id, false)?;
        let ((index, value), inputs) = Versions::capture(read);
        let slot = self.slot::<Option<(usize, Row<T>)>>(SlotId::Branch(id))?;
        let existing = {
            let state = slot.borrow();
            [&state.current, &state.candidate]
                .into_iter()
                .find_map(|entry| {
                    entry
                        .as_ref()
                        .filter(|(old, _)| *old == index)
                        .map(|(_, row)| row.clone())
                })
        };
        let (data, scope) = match existing {
            Some(row) => row,
            None => {
                let data = signal(value.clone());
                let scope = fusor::untrack(|| prepare(index, data.clone(), &self.tree.owner))?;
                (data, Rc::new(scope))
            }
        };
        data.with_render_value(Rc::new(value.clone()), inputs, || self.scope(&scope))?;
        self.publication
            .patches
            .push(Patch::Children(target, scope.roots.clone()));
        self.publish(
            slot,
            Some((index, (data.clone(), scope.clone()))),
            move || {
                data.set(value);
                scope.publish();
            },
        );
        Ok(())
    }
    pub fn keyed<T: Clone + PartialEq + 'static, K: Ord + Clone + 'static>(
        &mut self,
        id: usize,
        read: impl Fn() -> Vec<T>,
        key: impl Fn(&T) -> K,
        prepare: impl Fn(Signal<T>, &OwnerHandle) -> Result<Scope, Error>,
    ) -> Result<(), Error> {
        let target = self.target(id, true)?;
        let (values, inputs) = Versions::capture(read);
        let entries: Vec<_> = values
            .into_iter()
            .map(|value| (key(&value), value))
            .collect();
        if entries
            .iter()
            .map(|(key, _)| key)
            .collect::<BTreeSet<_>>()
            .len()
            != entries.len()
        {
            return Err("duplicate key in coherent list".into());
        }
        let slot = self.slot::<BTreeMap<K, Row<T>>>(SlotId::Rows(id))?;
        let mut next = BTreeMap::new();
        let mut roots = Vec::new();
        let mut updates = Vec::new();
        for (key, value) in entries {
            let existing = {
                let state = slot.borrow();
                state
                    .current
                    .get(&key)
                    .or_else(|| state.candidate.get(&key))
                    .cloned()
            };
            let (data, scope) = match existing {
                Some(row) => row,
                None => {
                    let data = signal(value.clone());
                    let scope = fusor::untrack(|| prepare(data.clone(), &self.tree.owner))?;
                    (data, Rc::new(scope))
                }
            };
            data.with_render_value(Rc::new(value.clone()), inputs.clone(), || {
                self.scope(&scope)
            })?;
            roots.extend(scope.roots.clone());
            updates.push((data.clone(), value, scope.clone()));
            next.insert(key, (data, scope));
        }
        self.publication
            .patches
            .push(Patch::Children(target, roots));
        self.publish(slot, next, move || {
            for (data, value, scope) in updates {
                data.set(value);
                scope.publish();
            }
        });
        Ok(())
    }
    pub fn children(&mut self, id: usize, children: Children) -> Result<(), Error> {
        let target = self.target(id, false)?;
        let slot = self.slot::<Option<Rc<Scope>>>(SlotId::Children(id))?;
        let existing = {
            let state = slot.borrow();
            state.current.clone().or_else(|| state.candidate.clone())
        };
        let next = match existing {
            Some(scope) => Some(scope),
            None => fusor::untrack(|| children.prepare(&self.tree.owner))?.map(Rc::new),
        };
        if let Some(scope) = &next {
            self.scope(scope)?;
        }
        self.publication.patches.push(Patch::Children(
            target,
            next.as_ref()
                .map_or_else(Vec::new, |scope| scope.roots.clone()),
        ));
        let activated = next.clone();
        self.publish(slot, next, move || {
            if let Some(scope) = activated {
                scope.publish();
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_candidates_retire_when_the_attempt_changes() {
        let mut parent = Scope::new(
            None,
            &[
                StaticNode {
                    parent: None,
                    kind: Kind::Element("main", &[], Some(0)),
                },
                StaticNode {
                    parent: Some(0),
                    kind: Kind::Mount(0),
                },
            ],
        )
        .unwrap();
        let boundary = AsyncBoundary::coherent();
        let gate = signal(true);
        let constructions = Rc::new(Cell::new(0));
        let cleanups = Rc::new(Cell::new(0));
        parent
            .async_region(0, boundary.clone(), {
                let (gate, constructions, cleanups) =
                    (gate.clone(), constructions.clone(), cleanups.clone());
                move |frame| {
                    if gate.get() {
                        frame.branch(
                            0,
                            || (0, ()),
                            |_, _, owner| {
                                constructions.set(constructions.get() + 1);
                                let mut scope = Scope::new(Some(owner), &[])?;
                                scope.set_coherent_renderer(|_| Ok(()));
                                let cleanups = cleanups.clone();
                                scope.retain_state(
                                    scope
                                        .owner()
                                        .on_cleanup(move || cleanups.set(cleanups.get() + 1)),
                                );
                                Ok(scope)
                            },
                        )?;
                    }
                    frame.attempt.pending();
                    Ok(())
                }
            })
            .unwrap();
        parent.publish();
        boundary.retry();
        assert_eq!((constructions.get(), cleanups.get()), (1, 0));
        gate.set(false);
        assert_eq!(
            cleanups.get(),
            1,
            "a skipped slot releases its obsolete candidate"
        );
        gate.set(true);
        assert_eq!(constructions.get(), 2);
        parent.dispose();
        assert_eq!(cleanups.get(), 2);
    }
}
