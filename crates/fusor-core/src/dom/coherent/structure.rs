use super::{Frame, RenderSlot, SlotId, Structure, allowed, error, visit};
use crate::dom::{Children, MountPoint, Scope, TemplateComponent};
use crate::{OwnerHandle, Signal, coherence::Attempt, signal, versions::Versions};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};
use wasm_bindgen::JsValue;
use web_sys::Element;

/// What a structural slot has published, and the candidate that the attempt
/// with `epoch` prepared. A newer attempt drops an older candidate first.
struct EpochSlot<V> {
    epoch: Cell<Option<u64>>,
    current: RefCell<V>,
    candidate: RefCell<V>,
}

impl<V: Default> Default for EpochSlot<V> {
    fn default() -> Self {
        Self {
            epoch: Cell::new(None),
            current: RefCell::default(),
            candidate: RefCell::default(),
        }
    }
}

impl<V: Default + 'static> EpochSlot<V> {
    fn begin(self: &Rc<Self>, attempt: &Attempt) {
        if self.epoch.replace(Some(attempt.epoch())) != Some(attempt.epoch()) {
            drop(self.candidate.take());
            let weak = Rc::downgrade(self);
            attempt.on_invalidate(move || {
                if let Some(slot) = weak.upgrade() {
                    slot.epoch.set(None);
                    drop(slot.candidate.take());
                }
            });
        }
    }

    fn propose(&self, next: V) {
        drop(self.candidate.replace(next));
    }

    /// Publish `next`. The caller decides when the previous value drops.
    fn publish(&self, next: V) -> V {
        let old = self.current.replace(next);
        drop(self.candidate.take());
        old
    }
}

impl<T: Clone> EpochSlot<Option<T>> {
    /// The published or proposed instance that `matches`, for reuse.
    fn find(&self, matches: impl Fn(&T) -> bool) -> Option<T> {
        [&self.current, &self.candidate]
            .into_iter()
            .find_map(|cell| {
                cell.borrow()
                    .as_ref()
                    .filter(|value| matches(value))
                    .cloned()
            })
    }
}

impl Frame<'_> {
    /// The typed state of `id`, created on first use.
    fn slot<S: Default + 'static>(&self, id: SlotId, changed: &str) -> Result<Rc<S>, String> {
        let state = self
            .tree
            .slots
            .borrow_mut()
            .entry(id)
            .or_insert_with(|| Rc::new(S::default()))
            .clone();
        state.downcast::<S>().map_err(|_| changed.into())
    }

    /// Validate a range and keep its anchors inside this component.
    fn range(&mut self, target: &MountPoint) -> Result<(), String> {
        target.validate().map_err(error)?;
        self.target(&target.start);
        self.target(&target.end);
        Ok(())
    }
}

impl Frame<'_> {
    pub fn component_at<C: TemplateComponent, K: PartialEq + 'static>(
        &mut self,
        RenderSlot {
            index: slot,
            target,
        }: RenderSlot<'_, MountPoint>,
        key: Option<K>,
        make: impl FnOnce(OwnerHandle) -> Result<C, JsValue>,
        children: Children,
    ) -> Result<(), String> {
        self.range(target)?;
        let state: Rc<ChildSlot<Rc<K>>> =
            self.slot(SlotId::Component(slot), "coherent child key type changed")?;
        state.begin(self.attempt);
        let next = match key.map(Rc::new) {
            None => None,
            Some(key) => {
                let instance = match state.find(|(old, _)| old == &key) {
                    Some((_, instance)) => instance,
                    None => {
                        let scope = children
                            .with(|| C::prepare(&self.tree.owner, make))
                            .map_err(error)?;
                        if scope.render_tree.is_none() {
                            return Err("a coherent child requires generated HTML bindings".into());
                        }
                        Rc::new(scope)
                    }
                };
                visit(
                    instance.render_tree.as_ref().expect("validated"),
                    self.attempt,
                    self.publication,
                )?;
                Some((key, instance))
            }
        };
        state.propose(next.clone());
        self.publication.structures.push(Box::new(ChildPlan {
            target: target.clone(),
            state,
            next,
        }));
        Ok(())
    }
}

type ChildSlot<K> = EpochSlot<Option<(K, Rc<Scope>)>>;
struct ChildPlan<K> {
    target: MountPoint,
    state: Rc<ChildSlot<K>>,
    next: Option<(K, Rc<Scope>)>,
}
impl<K: Clone + 'static> Structure for ChildPlan<K> {
    fn validate(&self) -> Result<(), String> {
        self.target.validate().map_err(error)?;
        if let Some((_, scope)) = self.state.current.borrow().as_ref() {
            scope.validate_nodes().map_err(error)?;
        }
        if let Some((_, scope)) = &self.next {
            scope.validate_nodes().map_err(error)?;
            if scope.owner().is_disposed() {
                return Err("prepared child was disposed".into());
            }
            // Widget setup is forbidden by generated coherent lowering.
            scope.finish_prepare().map_err(error)?;
        }
        Ok(())
    }
    fn apply(&self) -> Result<(), String> {
        let next = self.next.as_ref().map(|(_, scope)| scope);
        if let Some(next) = next.filter(|next| !self.target.precedes_end(next.last_node())) {
            next.insert_before(
                &self.target.parent_element().map_err(error)?,
                Some(&self.target.end),
            )
            .map_err(error)?;
        }
        if let Some((_, old)) = self.state.current.borrow().as_ref() {
            if next.is_none_or(|next| !Rc::ptr_eq(next, old)) {
                old.remove_nodes();
            }
        }
        Ok(())
    }
    fn finish(self: Box<Self>) {
        let old = self.state.publish(self.next.clone());
        if let Some((_, scope)) = &self.next {
            scope.commit();
        }
        drop(old);
    }
}

impl Frame<'_> {
    pub fn children_at(
        &mut self,
        slot: usize,
        target: &MountPoint,
        children: &Children,
    ) -> Result<(), String> {
        self.range(target)?;
        let state: Rc<ChildrenState> = self.slot(
            SlotId::Children(slot),
            "coherent children slot type changed",
        )?;
        state.begin(self.attempt);
        let scope = match state.find(|_| true) {
            Some(scope) => Some(scope),
            None => children
                .prepare(&self.tree.owner)
                .map_err(error)?
                .map(Rc::new),
        };
        if let Some(scope) = scope {
            visit(
                scope
                    .render_tree
                    .as_ref()
                    .ok_or("children require generated coherent HTML")?,
                self.attempt,
                self.publication,
            )?;
            state.propose(Some(scope.clone()));
            self.publication.structures.push(Box::new(ChildrenPlan {
                target: target.clone(),
                scope,
                state,
            }));
        }
        Ok(())
    }
}

type ChildrenState = EpochSlot<Option<Rc<Scope>>>;
struct ChildrenPlan {
    target: MountPoint,
    scope: Rc<Scope>,
    state: Rc<ChildrenState>,
}
impl Structure for ChildrenPlan {
    fn validate(&self) -> Result<(), String> {
        self.target.validate().map_err(error)?;
        self.scope.finish_prepare().map_err(error)
    }
    fn apply(&self) -> Result<(), String> {
        let fragment = self
            .scope
            .fragment
            .as_ref()
            .ok_or("missing children range")?;
        if !self.target.precedes_end(&fragment.end) {
            self.scope.attach_fragment(&self.target).map_err(error)?;
        }
        Ok(())
    }
    fn finish(self: Box<Self>) {
        let old = self.state.publish(Some(self.scope.clone()));
        self.scope.commit();
        drop(old);
    }
}

impl Frame<'_> {
    /// Compiler-owned exclusive fragment with reactive, branch-local captures.
    pub fn branch_at<T: Clone + PartialEq + 'static>(
        &mut self,
        slot: usize,
        target: &MountPoint,
        read: impl FnOnce() -> (usize, T),
        prepare: impl FnOnce(usize, Signal<T>, &OwnerHandle) -> Result<Scope, JsValue>,
    ) -> Result<(), String> {
        self.range(target)?;
        let ((key, data), inputs) = Versions::capture(read);
        let state: Rc<BranchSlot<T>> =
            self.slot(SlotId::Branch(slot), "coherent branch capture type changed")?;
        state.begin(self.attempt);
        let (value, scope) = match state.find(|(old, _, _)| *old == key) {
            Some((_, value, scope)) => (value, scope),
            None => {
                let value = signal(data.clone());
                let scope = prepare(key, value.clone(), &self.tree.owner).map_err(error)?;
                if scope.render_tree.is_none() {
                    return Err("coherent branches require generated HTML".into());
                }
                (value, Rc::new(scope))
            }
        };
        let data = Rc::new(data);
        value.with_render_value(data.clone(), inputs, || {
            visit(
                scope.render_tree.as_ref().expect("validated"),
                self.attempt,
                self.publication,
            )
        })?;
        let next = (key, value, scope);
        state.propose(Some(next.clone()));
        self.publication.structures.push(Box::new(BranchPlan {
            target: target.clone(),
            state,
            next,
            data,
        }));
        Ok(())
    }
}

type BranchInstance<T> = (usize, Signal<T>, Rc<Scope>);
type BranchSlot<T> = EpochSlot<Option<BranchInstance<T>>>;
struct BranchPlan<T> {
    target: MountPoint,
    state: Rc<BranchSlot<T>>,
    next: BranchInstance<T>,
    data: Rc<T>,
}
impl<T: Clone + PartialEq + 'static> Structure for BranchPlan<T> {
    fn validate(&self) -> Result<(), String> {
        self.target.validate().map_err(error)?;
        if self.next.2.owner().is_disposed() {
            return Err("prepared branch was disposed".into());
        }
        self.next.2.finish_prepare().map_err(error)
    }
    fn apply(&self) -> Result<(), String> {
        let fragment = self
            .next
            .2
            .fragment
            .as_ref()
            .ok_or("missing branch range")?;
        if !self.target.precedes_end(&fragment.end) {
            self.next.2.attach_fragment(&self.target).map_err(error)?;
        }
        if let Some((_, _, old)) = self.state.current.borrow().as_ref() {
            if !Rc::ptr_eq(old, &self.next.2) {
                old.fragment
                    .as_ref()
                    .ok_or("missing previous branch range")?
                    .remove();
            }
        }
        Ok(())
    }
    fn finish(self: Box<Self>) {
        let old = self.state.publish(Some(self.next.clone()));
        self.next.1.set((*self.data).clone());
        drop(old);
        self.next.2.commit();
    }
}

impl Frame<'_> {
    pub fn keyed<T, K>(
        &mut self,
        RenderSlot {
            index: slot,
            target: container,
        }: RenderSlot<'_, Element>,
        items: impl FnOnce() -> Vec<T>,
        key: impl Fn(&T) -> K,
        render: impl Fn(Signal<T>, &OwnerHandle) -> Result<Scope, JsValue>,
    ) -> Result<(), String>
    where
        T: Clone + PartialEq + 'static,
        K: Ord + Clone + 'static,
    {
        allowed(container)?;
        let (items, inputs) = Versions::capture(items);
        let keys: Vec<_> = items.iter().map(key).collect();
        if keys.iter().collect::<std::collections::BTreeSet<_>>().len() != keys.len() {
            return Err("duplicate key in coherent list".into());
        }
        let state: Rc<ListSlot<K, T>> =
            self.slot(SlotId::List(slot), "coherent list item/key type changed")?;
        state.begin(self.attempt);
        let mut next = BTreeMap::new();
        let mut updates = Vec::new();
        for (key, item) in keys.iter().zip(items) {
            let existing = state
                .current
                .borrow()
                .get(key)
                .cloned()
                .or_else(|| state.candidate.borrow().get(key).cloned());
            let (value, scope) = match existing {
                Some(existing) => existing,
                None => {
                    let value = signal(item.clone());
                    let scope = render(value.clone(), &self.tree.owner).map_err(error)?;
                    if scope.render_tree.is_none() {
                        return Err("coherent rows require generated HTML bindings".into());
                    }
                    (value, Rc::new(scope))
                }
            };
            let item = Rc::new(item);
            value.with_render_value(item.clone(), inputs.clone(), || {
                visit(
                    scope.render_tree.as_ref().expect("validated"),
                    self.attempt,
                    self.publication,
                )
            })?;
            updates.push((value.clone(), item));
            next.insert(key.clone(), (value, scope));
        }
        state.propose(next.clone());
        let remove = state
            .current
            .borrow()
            .iter()
            .filter(|(key, _)| !next.contains_key(*key))
            .map(|(_, (_, scope))| scope.clone())
            .collect();
        let order = keys.iter().map(|key| next[key].1.clone()).collect();
        self.publication.structures.push(Box::new(ListPlan {
            container: container.clone(),
            state,
            next,
            updates,
            remove,
            order,
        }));
        Ok(())
    }
}

type Rows<K, T> = BTreeMap<K, (Signal<T>, Rc<Scope>)>;
type ListSlot<K, T> = EpochSlot<Rows<K, T>>;
struct ListPlan<K, T> {
    container: Element,
    state: Rc<ListSlot<K, T>>,
    next: Rows<K, T>,
    updates: Vec<(Signal<T>, Rc<T>)>,
    remove: Vec<Rc<Scope>>,
    order: Vec<Rc<Scope>>,
}
impl<K: Ord + Clone + 'static, T: Clone + PartialEq + 'static> Structure for ListPlan<K, T> {
    fn validate(&self) -> Result<(), String> {
        for scope in &self.remove {
            scope.validate_nodes().map_err(error)?;
        }
        for (_, scope) in self.next.values() {
            scope.validate_nodes().map_err(error)?;
            scope.finish_prepare().map_err(error)?;
        }
        Ok(())
    }
    fn apply(&self) -> Result<(), String> {
        for root in &self.remove {
            root.remove_nodes();
        }
        let mut cursor = self.container.first_child();
        for root in &self.order {
            if !cursor
                .as_ref()
                .is_some_and(|node| node.is_same_node(Some(root.first_node())))
            {
                root.insert_before(&self.container, cursor.as_ref())
                    .map_err(error)?;
            }
            cursor = root.last_node().next_sibling();
        }
        Ok(())
    }
    fn finish(self: Box<Self>) {
        let old = self.state.publish(self.next.clone());
        // Rust publications and lifecycle callbacks occur after every DOM patch.
        for (value, item) in self.updates {
            value.set((*item).clone());
        }
        for (_, scope) in self.next.values() {
            scope.commit();
        }
        drop(old);
    }
}
