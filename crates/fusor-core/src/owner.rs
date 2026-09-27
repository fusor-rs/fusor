//! Explicit lifetimes shared by DOM scopes and optional runtime integrations.
use std::{
    any::{Any, TypeId},
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::{Rc, Weak},
};

type Callback = Box<dyn FnOnce()>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Prepared,
    Active,
    Disposed,
}

struct Inner {
    status: Cell<Status>,
    committed: Cell<bool>,
    // Monotone lifecycle history; disposal must not erase adopted-DOM ownership.
    activated: Cell<bool>,
    // Fallible DOM setup can finish while this owner still awaits activation.
    #[cfg(feature = "dom")]
    mount_ready: Cell<bool>,
    parent: Option<Weak<Inner>>,
    children: RefCell<Vec<Weak<Inner>>>,
    dead_children: Cell<usize>,
    next: Cell<u64>,
    activate: RefCell<BTreeMap<u64, Callback>>,
    cleanup: RefCell<BTreeMap<u64, Callback>>,
    contexts: RefCell<BTreeMap<TypeId, Rc<dyn Any>>>,
}

// Inner, rather than Owner, accounts for expiry: callbacks can temporarily
// retain an Inner while a unique Owner is being dropped.
impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(parent) = self.parent.as_ref().and_then(Weak::upgrade) {
            parent.dead_children.set(parent.dead_children.get() + 1);
        }
    }
}

/// A typed, namespaced key for a value supplied by a component or application.
/// Distinct key types can provide the same value type without colliding.
///
/// ```
/// use fusor::{ContextKey, Owner, Signal, signal};
/// struct Locale;
/// impl ContextKey for Locale { type Value = Signal<String>; }
/// let app = Owner::new();
/// app.handle().provide::<Locale>(signal("en".to_owned())).unwrap();
/// let child = Owner::child(&app.handle());
/// assert_eq!(child.handle().context::<Locale>().unwrap().get(), "en");
/// ```
pub trait ContextKey: 'static {
    type Value: 'static;
}

/// A context provider cannot be replaced or registered after disposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextError {
    Disposed,
    AlreadyProvided,
}

impl std::fmt::Display for ContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Disposed => "cannot provide context on a disposed owner",
            Self::AlreadyProvided => "this owner already provides the context key",
        })
    }
}

impl std::error::Error for ContextError {}

/// A unique lifetime owner. Commit starts registered work; drop disposes it.
/// Handles are weak and cannot extend the lifetime. This is single-threaded.
#[must_use = "dropping the owner disposes its work"]
pub struct Owner(Rc<Inner>);

/// Weak access to an owner's lifecycle. Safe to retain after disposal.
#[derive(Clone)]
pub struct OwnerHandle(Weak<Inner>);

/// Removing this registration unregisters the callback without invoking it.
#[must_use = "retain the registration until the callback is no longer needed"]
pub struct Registration {
    owner: Weak<Inner>,
    id: u64,
    activation: bool,
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade() {
            let removed = if self.activation {
                owner.activate.borrow_mut().remove(&self.id)
            } else {
                owner.cleanup.borrow_mut().remove(&self.id)
            };
            drop(removed);
        }
    }
}

impl Default for Owner {
    fn default() -> Self {
        Self::new()
    }
}

impl Owner {
    /// Prepare a root lifetime. Work waits for [`Self::commit`].
    pub fn new() -> Self {
        Self::create(None)
    }

    /// Prepare a child. It activates only after both it and its ancestors commit.
    /// An expired parent produces a disposed child.
    pub fn child(parent: &OwnerHandle) -> Self {
        Self::create(Some(parent.0.clone()))
    }

    fn create(parent: Option<Weak<Inner>>) -> Self {
        let owner = Self(Rc::new(Inner {
            status: Cell::new(Status::Prepared),
            committed: Cell::new(false),
            activated: Cell::new(false),
            #[cfg(feature = "dom")]
            mount_ready: Cell::new(false),
            parent,
            children: RefCell::new(Vec::new()),
            dead_children: Cell::new(0),
            next: Cell::new(0),
            activate: RefCell::new(BTreeMap::new()),
            cleanup: RefCell::new(BTreeMap::new()),
            contexts: RefCell::new(BTreeMap::new()),
        }));
        if let Some(parent) = &owner.0.parent {
            if let Some(parent) = parent
                .upgrade()
                .filter(|p| p.status.get() != Status::Disposed)
            {
                let mut children = parent.children.borrow_mut();
                // Reclaim weak tombstones only when they occupy at least half
                // the registry. Scanning on every insertion makes a wide tree
                // quadratic; this threshold amortizes scans over dead children.
                if parent.dead_children.get() >= 32
                    && parent.dead_children.get() >= children.len() / 2
                {
                    children.retain(|child| child.strong_count() != 0);
                    parent.dead_children.set(0);
                }
                children.push(Rc::downgrade(&owner.0));
            } else {
                owner.0.status.set(Status::Disposed);
            }
        }
        owner
    }

    pub fn handle(&self) -> OwnerHandle {
        OwnerHandle(Rc::downgrade(&self.0))
    }

    /// Commit once preparation succeeds. Idempotent; cannot revive disposal.
    pub fn commit(&self) {
        self.0.committed.set(true);
        activate(&self.0);
    }

    /// The status of an owner this caller holds, without a weak handle.
    #[cfg(feature = "dom")]
    pub(crate) fn is_active(&self) -> bool {
        self.0.status.get() == Status::Active
    }

    #[cfg(feature = "dom")]
    pub(crate) fn is_disposed(&self) -> bool {
        self.0.status.get() == Status::Disposed
    }

    #[cfg(feature = "dom")]
    pub(crate) fn mark_mount_ready(&self) {
        self.0.mount_ready.set(true);
    }

    #[cfg(any(feature = "dom", test))]
    pub(crate) fn was_activated(&self) -> bool {
        self.0.activated.get()
    }

    /// Invalidate the whole subtree before calling any cleanup. Reentrant and
    /// idempotent. Cleanup is synchronous; it cannot await task termination.
    pub fn dispose(&self) {
        let mut callbacks = Vec::new();
        invalidate(&self.0, &mut callbacks);
        for callback in callbacks {
            callback();
        }
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.dispose();
    }
}

fn activate(inner: &Rc<Inner>) {
    if inner.status.get() != Status::Prepared || !inner.committed.get() {
        return;
    }
    if let Some(parent) = &inner.parent {
        if !parent
            .upgrade()
            .is_some_and(|p| p.status.get() == Status::Active)
        {
            return;
        }
    }
    inner.status.set(Status::Active);
    inner.activated.set(true);
    let callbacks = inner.activate.take();
    for callback in callbacks.into_values() {
        if inner.status.get() == Status::Active {
            callback();
        }
    }
    let children = inner.children.borrow().clone();
    for child in children.into_iter().filter_map(|child| child.upgrade()) {
        activate(&child);
    }
}

fn invalidate(inner: &Rc<Inner>, callbacks: &mut Vec<Callback>) {
    if inner.status.replace(Status::Disposed) == Status::Disposed {
        return;
    }
    // Most owners register nothing; skip their empty registries.
    // Hold discarded activation callbacks until the entire tree is invalid.
    if !inner.activate.borrow().is_empty() {
        let activations = inner.activate.take();
        callbacks.push(Box::new(move || drop(activations)));
    }
    if !inner.cleanup.borrow().is_empty() {
        callbacks.extend(inner.cleanup.take().into_values());
    }
    if !inner.children.borrow().is_empty() {
        for child in inner
            .children
            .take()
            .into_iter()
            .filter_map(|c| c.upgrade())
        {
            invalidate(&child, callbacks);
        }
    }
    // Defer destructors until the whole tree is invalid. Release a provider
    // after its children's cleanup callbacks have run.
    if !inner.contexts.borrow().is_empty() {
        let contexts = inner.contexts.take();
        callbacks.push(Box::new(move || drop(contexts)));
    }
}

impl OwnerHandle {
    /// Wrap a callback with a weak lifetime check. Late external callbacks return
    /// `None` after disposal (or before activation) instead of publishing state.
    /// The callback's own captures still obey ordinary Rust ownership rules.
    pub fn guarded<A, R, F: FnMut(A) -> R>(
        &self,
        mut callback: F,
    ) -> impl FnMut(A) -> Option<R> + use<A, R, F> {
        let owner = self.clone();
        move |argument| {
            owner
                .is_active()
                .then(|| crate::untrack(|| callback(argument)))
        }
    }
    /// Supply a value once on this owner. Descendants resolve the nearest key.
    /// This is not reactive registration: supply a signal for changing state.
    pub fn provide<K: ContextKey>(&self, value: K::Value) -> Result<(), ContextError> {
        let Some(inner) = self
            .0
            .upgrade()
            .filter(|inner| inner.status.get() != Status::Disposed)
        else {
            return Err(ContextError::Disposed);
        };
        // A rejected value is dropped after this local registry borrow.
        let mut contexts = inner.contexts.borrow_mut();
        if contexts.contains_key(&TypeId::of::<K>()) {
            return Err(ContextError::AlreadyProvided);
        }
        contexts.insert(TypeId::of::<K>(), Rc::new(value));
        Ok(())
    }

    /// Find the nearest live provider, including this owner. Missing/disposed
    /// context returns `None`. Lookup is untracked and does not keep owners alive.
    /// An obtained `Rc` can outlive the provider like any ordinary Rust value;
    /// disposal prevents further lookup and releases the owner's reference.
    pub fn context<K: ContextKey>(&self) -> Option<Rc<K::Value>> {
        let mut current = self.0.upgrade();
        while let Some(inner) = current {
            if inner.status.get() == Status::Disposed {
                return None;
            }
            let value = inner.contexts.borrow().get(&TypeId::of::<K>()).cloned();
            if let Some(value) = value {
                return Some(
                    value
                        .downcast::<K::Value>()
                        .expect("context key and value type agree"),
                );
            }
            current = inner.parent.as_ref().and_then(Weak::upgrade);
        }
        None
    }

    /// Whether this is an immediate child of the given lifetime.
    pub fn is_child_of(&self, parent: &Self) -> bool {
        self.0
            .upgrade()
            .is_some_and(|inner| inner.parent.as_ref().is_some_and(|p| p.ptr_eq(&parent.0)))
    }
    #[cfg(feature = "dom")]
    pub(crate) fn is_within(&self, ancestor: &Self) -> bool {
        let mut cursor = Some(self.0.clone());
        while let Some(owner) = cursor {
            if owner.ptr_eq(&ancestor.0) {
                return true;
            }
            cursor = owner.upgrade().and_then(|owner| owner.parent.clone());
        }
        false
    }
    pub fn is_active(&self) -> bool {
        self.0
            .upgrade()
            .is_some_and(|p| p.status.get() == Status::Active)
    }
    pub fn is_disposed(&self) -> bool {
        self.0
            .upgrade()
            .is_none_or(|p| p.status.get() == Status::Disposed)
    }
    #[cfg(feature = "dom")]
    pub(crate) fn is_mount_ready(&self) -> bool {
        self.0.upgrade().is_some_and(|p| p.mount_ready.get())
    }
    /// Run on activation (immediately if active). Does not run after disposal.
    pub fn on_activate(&self, callback: impl FnOnce() + 'static) -> Registration {
        self.register(Box::new(callback), true)
    }
    /// Run once on disposal (immediately if already disposed).
    pub fn on_cleanup(&self, callback: impl FnOnce() + 'static) -> Registration {
        self.register(Box::new(callback), false)
    }
    fn register(&self, callback: Callback, activation: bool) -> Registration {
        let mut registration = Registration {
            owner: self.0.clone(),
            id: 0,
            activation,
        };
        let Some(inner) = self.0.upgrade() else {
            if !activation {
                callback();
            }
            return registration;
        };
        let id = inner
            .next
            .get()
            .checked_add(1)
            .expect("owner registration overflow");
        inner.next.set(id);
        match (activation, inner.status.get()) {
            (true, Status::Active) | (false, Status::Disposed) => callback(),
            (true, Status::Disposed) => {}
            (true, Status::Prepared) => {
                inner.activate.borrow_mut().insert(id, callback);
            }
            (false, _) => {
                inner.cleanup.borrow_mut().insert(id, callback);
            }
        }
        registration.id = id;
        registration
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn transient_children_are_compacted_without_losing_survivors_or_order() {
        let parent = Owner::new();
        let first = Owner::child(&parent.handle());
        for _ in 0..10_000 {
            drop(Owner::child(&parent.handle()));
        }
        let last = Owner::child(&parent.handle());
        assert!(parent.0.children.borrow().len() <= 34);
        let calls = Rc::new(RefCell::new(Vec::new()));
        let registrations: Vec<_> = [&first, &last]
            .into_iter()
            .enumerate()
            .map(|(id, owner)| {
                let calls = calls.clone();
                owner.commit();
                owner
                    .handle()
                    .on_activate(move || calls.borrow_mut().push(id))
            })
            .collect();
        parent.commit();
        assert_eq!(*calls.borrow(), [0, 1]);
        parent.dispose();
        assert!(first.handle().is_disposed());
        assert!(last.handle().is_disposed());
        drop(registrations);
    }
}

#[cfg(test)]
mod activation_history_tests {
    use super::*;

    #[test]
    fn activation_history_survives_disposal_and_does_not_confuse_commit_with_activation() {
        let parent = Owner::new();
        let child = Owner::child(&parent.handle());
        assert!(!parent.was_activated());
        child.commit();
        assert!(!child.was_activated());
        parent.commit();
        assert!(parent.was_activated());
        assert!(child.was_activated());
        parent.dispose();
        assert!(parent.was_activated());
        assert!(child.was_activated());
        child.commit();
        assert!(child.was_activated());
        assert!(child.handle().is_disposed());
    }

    #[test]
    fn failed_preparation_and_disposed_ancestors_never_claim_activation() {
        let parent = Owner::new();
        let child = Owner::child(&parent.handle());
        child.commit();
        parent.dispose();
        assert!(!parent.was_activated());
        assert!(!child.was_activated());
        let late = Owner::child(&parent.handle());
        late.commit();
        assert!(!late.was_activated());
        let orphan = Owner::child(&Owner::new().handle());
        orphan.commit();
        assert!(!orphan.was_activated());
    }

    #[test]
    fn parent_disposal_during_activation_does_not_claim_its_waiting_child() {
        let parent = Rc::new(Owner::new());
        let child = Owner::child(&parent.handle());
        child.commit();
        let weak = Rc::downgrade(&parent);
        let _registration = parent.handle().on_activate(move || {
            weak.upgrade().unwrap().dispose();
        });
        parent.commit();
        assert!(parent.was_activated());
        assert!(!child.was_activated());
        assert!(child.handle().is_disposed());
    }

    #[test]
    fn history_is_set_before_self_disposal_in_an_activation_callback() {
        let owner = Rc::new(Owner::new());
        let weak = Rc::downgrade(&owner);
        let observed = Rc::new(Cell::new(false));
        let observed_in_callback = observed.clone();
        let _registration = owner.handle().on_activate(move || {
            let owner = weak.upgrade().unwrap();
            observed_in_callback.set(owner.was_activated());
            owner.dispose();
        });
        owner.commit();
        assert!(observed.get());
        assert!(owner.was_activated());
        assert!(owner.handle().is_disposed());
    }
}
