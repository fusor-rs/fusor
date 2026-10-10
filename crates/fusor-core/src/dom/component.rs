//! Structural bindings that show one child view at a time and keep it while
//! its identity is unchanged. Construction runs once per identity, untracked.
use super::{Children, JsValue, MountPoint, Scope, TemplateComponent, hydration};
use crate::{OwnerHandle, untrack};

/// The child view a structural binding shows now, and the identity it was
/// prepared for.
pub(super) struct Retained<K>(Option<(K, Scope)>);

impl<K> Default for Retained<K> {
    fn default() -> Self {
        Self(None)
    }
}

impl<K> Retained<K> {
    pub(super) fn key(&self) -> Option<&K> {
        self.0.as_ref().map(|(key, _)| key)
    }

    pub(super) fn clear(&mut self) {
        self.0 = None;
    }

    /// Show a prepared `child` in place of the current one. `insert` places it
    /// in the document, then its fallible setup runs; if either fails, `child`
    /// is dropped with its DOM and the current view stays. Otherwise the
    /// current view is dropped before `child` activates, so a setup error
    /// during activation, which is logged, leaves the disposed replacement in
    /// the slot until its next replacement or disposal.
    pub(super) fn replace(
        &mut self,
        key: K,
        mut child: Scope,
        insert: impl FnOnce(&mut Scope) -> Result<(), JsValue>,
    ) -> Result<(), JsValue> {
        insert(&mut child)?;
        child.finish_prepare()?;
        self.0.insert((key, child)).1.commit();
        Ok(())
    }
}

impl Scope {
    /// Compiler entry point for a component tag. Keeps one child in `target`
    /// per identity: an unchanged identity retains its DOM and local state,
    /// `None` disposes the child, and a new identity prepares a replacement
    /// before the previous child is dropped. Construction and mounting are
    /// untracked; pass signals as inputs to update a retained instance.
    /// `children` reaches the child's generated prepare through
    /// [`Children::with`].
    #[doc(hidden)]
    pub fn component_at<C, K>(
        &mut self,
        target: &MountPoint,
        identity: impl Fn() -> Option<K> + 'static,
        make: impl Fn(OwnerHandle) -> Result<C, JsValue> + 'static,
        children: Children,
    ) -> Result<(), JsValue>
    where
        C: TemplateComponent,
        K: PartialEq + 'static,
    {
        let target = target.clone();
        let parent = self.owner();
        let mut hydrating = self.is_hydrating();
        let mut current = Retained::default();
        // Runs synchronously once here, so only that first run can adopt
        // server-rendered DOM; later identities are always built detached.
        self.bind(move || {
            let next = identity();
            untrack(|| {
                let server = if std::mem::take(&mut hydrating) {
                    target.hydrated_root(next.is_some())?
                } else {
                    None
                };
                let Some(key) = next else {
                    current.clear();
                    return Ok(());
                };
                if current.key() == Some(&key) {
                    return Ok(());
                }
                let child = hydration::with_target(server.clone(), || {
                    children.with(|| C::prepare(&parent, &make))
                })?;
                // A generated mount adopts the server root in place. Anything
                // else, such as a hand-written component, replaces it.
                let adopted = server
                    .as_ref()
                    .is_some_and(|target| target.adopted_by(&child));
                current.replace(key, child, |child| {
                    if adopted {
                        // Removal on drop still waits for hydration ownership.
                        child.remove_on_drop = true;
                        Ok(())
                    } else {
                        child.attach_at(&target)
                    }
                })?;
                if let Some(stale) = server.filter(|_| !adopted) {
                    stale.remove();
                }
                Ok(())
            })
        })
    }
}
