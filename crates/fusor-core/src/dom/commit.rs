//! Fallible browser setup runs after the complete prepared subtree is valid,
//! before activating resources. Child commits join their prepared ancestor.
use super::{JsValue, Scope};
use crate::{ContextKey, OwnerHandle};
use std::cell::RefCell;

struct Action {
    owner: OwnerHandle,
    setup: Box<dyn FnOnce() -> Result<(), JsValue>>,
}

#[derive(Default)]
pub(super) struct CommitQueue(RefCell<Vec<Action>>);

impl CommitQueue {
    /// No scope sharing this queue has setup left to finish.
    pub(super) fn is_idle(&self) -> bool {
        self.0.borrow().is_empty()
    }
}

struct MountContext;
impl ContextKey for MountContext {
    type Value = CommitQueue;
}

impl Scope {
    pub(super) fn prepare_queue(&mut self, parent: Option<&OwnerHandle>) {
        self.mount_parent = parent.cloned();
        if self.owner.is_disposed() {
            self.mount_queue = None;
            return;
        }
        let queue = parent
            .and_then(|parent| parent.context::<MountContext>())
            .unwrap_or_else(|| {
                let owner = self.owner();
                owner
                    .provide::<MountContext>(CommitQueue::default())
                    .expect("fresh component owner");
                owner.context::<MountContext>().expect("just provided")
            });
        self.mount_queue = Some(queue);
    }

    /// Internal integration boundary for fallible browser setup. The callback
    /// must arrange rollback on owner disposal if later setup fails. Captures
    /// should not keep a disposed component or integration alive.
    #[doc(hidden)]
    pub fn before_commit(
        &mut self,
        setup: impl FnOnce() -> Result<(), JsValue> + 'static,
    ) -> Result<(), JsValue> {
        if self.owner.is_disposed() {
            return Err(JsValue::from_str("cannot initialize a disposed component"));
        }
        if self.owner.is_active() {
            return setup();
        }
        let queue = self.mount_queue.as_ref().expect("prepared component queue");
        queue.0.borrow_mut().push(Action {
            owner: self.owner(),
            setup: Box::new(setup),
        });
        Ok(())
    }

    /// Finish fallible setup without activating the component's resources.
    /// Used before replacing a previous view.
    #[doc(hidden)]
    pub fn finish_prepare(&self) -> Result<(), JsValue> {
        if self.owner.is_disposed() {
            return Err(JsValue::from_str("cannot commit a disposed component"));
        }
        self.owner.mark_mount_ready();
        if self
            .mount_parent
            .as_ref()
            .is_some_and(|parent| !parent.is_active())
        {
            return Ok(());
        }
        self.finish_prepare_subtree()
    }

    /// Finish this prepared subtree inside an externally owned transaction.
    /// Unlike finish_prepare, an inactive parent does not defer fallible setup.
    /// The caller must keep work inactive until all participants can commit.
    pub fn finish_prepare_subtree(&self) -> Result<(), JsValue> {
        if self.owner.is_disposed() {
            return Err(JsValue::from_str("cannot prepare a disposed subtree"));
        }
        self.owner.mark_mount_ready();
        if let Some(queue) = &self.mount_queue {
            let pending = queue.0.take();
            let owner = self.owner();
            let (ready, waiting): (Vec<_>, Vec<_>) = pending
                .into_iter()
                .filter(|action| !action.owner.is_disposed())
                .partition(|action| {
                    action.owner.is_within(&owner) && action.owner.is_mount_ready()
                });
            queue.0.borrow_mut().extend(waiting);
            for action in ready {
                if !action.owner.is_disposed() {
                    (action.setup)()?;
                }
            }
        }
        if self.owner.is_disposed() {
            return Err(JsValue::from_str(
                "component disposed during initialization",
            ));
        }
        Ok(())
    }

    /// Finish browser setup and activate owned work. Prepared ancestors still
    /// gate activation. Failure is returned to the mounting caller.
    pub fn try_commit(&self) -> Result<(), JsValue> {
        // Reconciliation revisits retained scopes. An active scope has already
        // prepared and committed, but new descendants can share its setup queue.
        // Only skip preparation when there is no pending setup anywhere in it.
        if self.owner.is_active()
            && self
                .mount_queue
                .as_ref()
                .is_none_or(|queue| queue.0.borrow().is_empty())
        {
            return Ok(());
        }
        self.finish_prepare()?;
        self.owner.commit();
        if self.owner.is_disposed() {
            return Err(JsValue::from_str("component disposed during activation"));
        }
        Ok(())
    }
}
