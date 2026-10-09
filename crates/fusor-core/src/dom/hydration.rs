//! Hand server-rendered DOM from a structural binding to the generated mount
//! it prepares next. Generated mounts consume the offer before constructing
//! state and reject a mismatched shape. The previous offer returns afterwards.
use super::{JsValue, MountPoint, Scope, scoped};
use std::cell::RefCell;
use web_sys::Element;

#[derive(Clone)]
pub(super) enum Target {
    /// The root element of a component, adopted by a root-template mount.
    Root(Element),
    /// The range holding a fragment, adopted by a fragment mount.
    Range(MountPoint),
    Slot(MountPoint),
}

thread_local! {
    static OFFER: RefCell<Option<Target>> = const { RefCell::new(None) };
}

#[cfg(feature = "islands")]
pub(super) fn with_root<R>(root: &Element, run: impl FnOnce() -> R) -> R {
    scoped(&OFFER, Some(Target::Root(root.clone())), run)
}

/// `None` withdraws any outer offer while `run` prepares detached DOM.
pub(super) fn with_range<R>(range: Option<MountPoint>, run: impl FnOnce() -> R) -> R {
    scoped(&OFFER, range.map(Target::Slot), run)
}

pub(super) fn with_target<R>(target: Option<Target>, run: impl FnOnce() -> R) -> R {
    scoped(&OFFER, target, run)
}

pub(super) fn take_root() -> Result<Option<Element>, JsValue> {
    OFFER.with_borrow_mut(|offer| match offer.take() {
        Some(Target::Root(root)) => Ok(Some(root)),
        None => Ok(None),
        _ => Err(JsValue::from_str(
            "server component identity/shape mismatch: expected one root element",
        )),
    })
}

pub(super) fn take_range() -> Result<Option<MountPoint>, JsValue> {
    OFFER.with_borrow_mut(|offer| match offer.take() {
        Some(Target::Range(range)) => Ok(Some(range)),
        Some(Target::Slot(slot)) => slot.fragment().map(Some),
        None => Ok(None),
        _ => Err(JsValue::from_str(
            "server component identity/shape mismatch: expected a fragment",
        )),
    })
}

impl Target {
    pub(super) fn adopted_by(&self, scope: &Scope) -> bool {
        match self {
            Self::Root(root) => scope.fragment.is_none() && scope.root.is_same_node(Some(root)),
            Self::Range(range) => scope.fragment.as_ref().is_some_and(|child| {
                child.start.is_same_node(Some(&range.start))
                    && child.end.is_same_node(Some(&range.end))
            }),
            Self::Slot(_) => false,
        }
    }

    pub(super) fn remove(&self) {
        match self {
            Self::Root(root) => super::remove_tree(root),
            Self::Range(range) => range.remove(),
            Self::Slot(_) => unreachable!("only native views are removed"),
        }
    }
}
