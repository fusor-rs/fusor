//! Lazy, typed HTML content. Factories are reusable; mounted scopes are unique.
use super::{ElementTarget, JsValue, Scope, TemplateComponent, component::Retained};
use crate::{OwnerHandle, untrack};
use std::rc::Rc;

type Factory = dyn Fn(&OwnerHandle) -> Result<Scope, JsValue>;

/// A reusable factory for an authored HTML template component.
///
/// Clone shares factory identity, never DOM or a mounted lifetime. Each slot
/// prepares a fresh child of its receiving component; captured Rust values keep
/// their ordinary lexical scope. Construct content once and pass clones to slots.
/// Making a new factory in a reactive slot expression deliberately replaces it.
#[derive(Clone)]
#[must_use = "content is lazy; supply it to a slot to mount it"]
pub struct Content(Rc<Factory>);

impl Content {
    pub fn new<C: TemplateComponent>(make: impl Fn(OwnerHandle) -> C + 'static) -> Self {
        Self::try_new(move |owner| Ok(make(owner)))
    }

    /// Fallible constructor; owner-activation registrations wait for the receiving
    /// scope's commit. Ordinary constructor effects retain their immediate timing.
    pub fn try_new<C: TemplateComponent>(
        make: impl Fn(OwnerHandle) -> Result<C, JsValue> + 'static,
    ) -> Self {
        Self(Rc::new(move |parent| C::prepare(parent, &make)))
    }

    /// Compiler contract for inline content with an inferred parent state type.
    /// The normal detached-child checks still run in `prepare`.
    #[doc(hidden)]
    pub fn from_prepared(make: impl Fn(&OwnerHandle) -> Result<Scope, JsValue> + 'static) -> Self {
        Self(Rc::new(make))
    }

    /// Prepare a fresh, detached template child for an outlet or custom host.
    /// The caller attaches it and commits its scope; activation registrations wait
    /// for commit. Ordinary constructor effects retain their immediate timing.
    pub fn prepare(&self, parent: &OwnerHandle) -> Result<Scope, JsValue> {
        let scope = (self.0)(parent)?;
        let owner = scope.owner();
        if owner.is_disposed()
            || owner.is_active()
            || !owner.is_child_of(parent)
            || scope.root().parent_node().is_some()
        {
            return Err(JsValue::from_str(
                "fusor: slot content must be a detached, prepared child of its receiving owner",
            ));
        }
        Ok(scope)
    }
}

impl PartialEq for Content {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Content {}

impl Scope {
    /// Mount optional content in an empty host. An unchanged factory retains its
    /// DOM and local state; `None` removes it. Read signals inside `read` to select
    /// content reactively. Each host owns an independent instance.
    pub fn slot<R: Into<Option<Content>>>(
        &mut self,
        target: impl ElementTarget,
        read: impl Fn() -> R + 'static,
    ) -> Result<(), JsValue> {
        self.slot_with(target, move || read().into().map(|content| ((), content)))
    }

    /// Like [`Self::slot`], with an explicit reset key in addition to factory
    /// identity. Failed preparation leaves the previous instance alive.
    pub fn slot_with<K: PartialEq + 'static>(
        &mut self,
        target: impl ElementTarget,
        read: impl Fn() -> Option<(K, Content)> + 'static,
    ) -> Result<(), JsValue> {
        let container = target.resolve(self)?;
        let parent = self.owner();
        let mut current: Retained<(K, Content)> = Retained::default();
        self.bind(move || {
            let next = read();
            untrack(|| {
                let Some(identity) = next else {
                    current.clear();
                    return Ok(());
                };
                if current.key() == Some(&identity) {
                    return Ok(());
                }
                let child = identity.1.prepare(&parent)?;
                current.replace(identity, child, |child| child.attach(&container))
            })
        })
    }
}
