//! Shared integration machinery for independently maintained renderers.
use crate::OwnerHandle;
use std::{
    any::{Any, TypeId},
    cell::RefCell,
    collections::BTreeMap,
    rc::Rc,
};

/// Version of the renderer integration contract.
pub const VERSION: u32 = 1;

/// The construction handoff used by generated components. Mounting, publication
/// and cleanup remain renderer responsibilities; state must outlive its bindings.
pub trait Scope {
    fn owner(&self) -> OwnerHandle;
    fn retain_state<T: 'static>(&mut self, state: T) -> Rc<T>;

    /// Explicit candidate preparation defers constructor effects until activation.
    /// Ordinary mounting preserves their immediate execution.
    fn prepares_effects(&self) -> bool {
        false
    }
}

/// Construct and retain state after the renderer has validated its template.
/// This neither activates the owner nor publishes the scope. Callers control
/// reactive tracking around preparation and dispose failed candidate scopes.
pub fn construct<S: Scope, T: 'static, E>(
    scope: &mut S,
    make: impl FnOnce(OwnerHandle) -> Result<T, E>,
) -> Result<Rc<T>, E> {
    let state = if scope.prepares_effects() {
        crate::coherence::prepare_state(scope.owner(), make)?
    } else {
        make(scope.owner())?
    };
    Ok(scope.retain_state(state))
}

type Factory<S, E> = dyn Fn(&OwnerHandle) -> Result<S, E>;

/// A captured child fragment. Its renderer supplies placement and ownership.
pub struct Children<S, E>(Option<Rc<Factory<S, E>>>);

impl<S, E> Clone for Children<S, E> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S, E> Default for Children<S, E> {
    fn default() -> Self {
        Self(None)
    }
}

thread_local! {
    static INCOMING: RefCell<BTreeMap<TypeId, Box<dyn Any>>> = const { RefCell::new(BTreeMap::new()) };
}

impl<S: 'static, E: 'static> Children<S, E> {
    pub fn new(make: impl Fn(&OwnerHandle) -> Result<S, E> + 'static) -> Self {
        Self(Some(Rc::new(make)))
    }

    /// Take the children supplied to the component currently being constructed.
    pub fn take() -> Self {
        Self::replace(None).unwrap_or_default()
    }

    /// Supply children during construction, restoring the previous context on unwind.
    /// Different scope/error types have independent contexts.
    pub fn with<R>(&self, run: impl FnOnce() -> R) -> R {
        struct Restore<S: 'static, E: 'static>(Option<Children<S, E>>);
        impl<S: 'static, E: 'static> Drop for Restore<S, E> {
            fn drop(&mut self) {
                drop(Children::replace(self.0.take()));
            }
        }
        let _restore = Restore(Self::replace(self.0.as_ref().map(|_| self.clone())));
        run()
    }

    pub fn prepare(&self, parent: &OwnerHandle) -> Result<Option<S>, E> {
        self.0.as_ref().map(|make| make(parent)).transpose()
    }

    fn replace(value: Option<Self>) -> Option<Self> {
        let previous = INCOMING.with_borrow_mut(|incoming| match value {
            Some(value) => incoming.insert(TypeId::of::<Self>(), Box::new(value)),
            None => incoming.remove(&TypeId::of::<Self>()),
        });
        // Factory captures may reenter rendering when dropped; release the map first.
        previous.map(|value| *value.downcast::<Self>().expect("children context type"))
    }
}
